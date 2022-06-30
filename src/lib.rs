use bundle::Bundles;
use encoding_rs::GBK;
use time::{format_description, PrimitiveDateTime};

pub mod crypto;

mod lua;

mod bundle;

#[derive(Debug, Clone, Default)]
pub struct GameRes<'a, 'b, 'c> {
    keywords: Option<&'a str>,
    database: Option<&'b [u8]>,
    statistics: bool,
    build_time: Option<PrimitiveDateTime>,
    filename: Option<&'c str>,
}

impl<'a, 'b, 'c> GameRes<'a, 'b, 'c> {
    pub fn new() -> Self {
        GameRes {
            ..Default::default()
        }
    }

    pub fn illegal_keywords(mut self, keywords: &'a str) -> Self {
        self.keywords = Some(keywords);
        self
    }

    pub fn anti_memory_cheat(self, switch: bool) -> Self {
        assert!(switch);
        self
    }

    pub fn statistics(mut self, switch: bool) -> Self {
        self.statistics = switch;
        self
    }

    pub fn build_time(mut self, time: PrimitiveDateTime) -> Self {
        self.build_time = Some(time);
        self
    }

    pub fn filename(mut self, name: &'c str) -> Self {
        self.filename = Some(name);
        self
    }

    pub fn anti_speed_hack(self, switch: bool) -> Self {
        assert!(switch);
        self
    }

    pub fn game_lua(mut self, data: &'b [u8]) -> Self {
        self.database = Some(data);
        self
    }

    fn create_adaptor(&self) -> Vec<u8> {
        let time_fmt =
            format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]").unwrap();

        let time = self.build_time.unwrap().format(&time_fmt).unwrap();

        let s = format!(
            r#"
        local f1 = 核心.数据统计
        核心.数据统计 = function(uid, gid, unk, hash, time)
            if {enable_statistics} then
                f1({uid}, {gid}, 1, "{hash}", "{time}")
            end
        end
        local f2 = 核心.anti_hacking
        核心.anti_hacking = function(enabled, keywords)
            f2(1, "{keywords}")
        end
        "#,
            enable_statistics = self.statistics,
            uid = 0,
            gid = 0,
            hash = self.filename.unwrap(),
            time = time,
            keywords = self.keywords.unwrap_or_default()
        );

        let (b, _, _) = GBK.encode(&s);
        b.into_owned()
    }

    pub fn build(&self) -> Result<Vec<u8>, mlua::Error> {
        let database = self.database.expect("database should set");
        // check if database too small
        if database.len() < 0x200 {
            return Err(mlua::Error::external("invalid database"));
        }

        // trim plugin info header
        let (_header, database) = database.split_at(0x200);

        // compile database to bytecode
        let database = lua::compile("database.lua", database)?;

        // insert bundled library adaptor
        let adaptor = self.create_adaptor();

        let adaptor = lua::compile("adaptor.lua", adaptor)?;
        // build bundles
        let mut bundles = Bundles::with_adaptor(adaptor);
        bundles.set_database(database);
        let packed = bundles.pack()?;

        Ok(packed)
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use mlua::Lua;

    use super::*;

    #[test]
    fn extract_bundle() {
        let mut chunk = std::fs::read("").unwrap();
        chunk.truncate(chunk.len() - 10);
        crypto::decrypt_res(&mut chunk);

        let mut entries = IndexMap::new();

        let lua = unsafe { Lua::unsafe_new() };
        lua.scope(|s| {
            let dummy = s.create_function_mut(|_, (name, data): (String, String)| {
                let name = hex::decode(name).map_err(mlua::Error::external)?;
                let (name, _, _) = GBK.decode(&name);

                let mut lua = Vec::new();
                let mut data = hex::decode(data).map_err(mlua::Error::external)?;
                crypto::decrypt_ulib(&mut data);
                let _ = crypto::decompress(&data, &mut lua);

                entries.insert(name.into_owned(), lua);
                Ok(())
            })?;
            lua.globals().set("__U_Lib", dummy)?;
            lua.load(&chunk).exec()
        })
        .unwrap();
    }
}
