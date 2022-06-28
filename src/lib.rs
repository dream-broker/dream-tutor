use bundle::Bundles;
use encoding_rs::GBK;
use time::{format_description, OffsetDateTime};

pub mod crypto;

mod lua {
    use bstr::BString;
    use mlua::Lua;

    pub fn compile(chunk: impl AsRef<[u8]>) -> Result<Vec<u8>, mlua::Error> {
        let lua = unsafe { Lua::unsafe_new() };

        let f = lua.load(chunk.as_ref()).into_function().unwrap();

        let data: BString = lua
            .load("return string.dump")
            .eval::<mlua::Function>()
            .unwrap()
            .call(f)?;
        Ok(data.into())
    }
}

mod bundle {
    use encoding_rs::GBK;
    use indexmap::IndexMap;
    use std::fmt::Write;

    use crate::{crypto, lua};

    const BUILDIN_BUNDLED_LIBRARY: &[u8] = &[];

    pub struct Bundles {
        entries: IndexMap<&'static str, Vec<u8>>,
    }

    impl Bundles {
        pub fn with_adaptor(adaptor: Vec<u8>) -> Self {
            let mut entries = IndexMap::with_capacity(3);
            entries.insert("adaptor.lua", adaptor);
            entries.insert("bundles.lua", BUILDIN_BUNDLED_LIBRARY.to_owned());
            Self { entries }
        }

        pub fn pack(&self) -> Result<Vec<u8>, mlua::Error> {
            let mut s = String::new();
            for (name, lua) in &self.entries {
                let (name, _, _) = GBK.encode(name);
                let name = hex::encode_upper(name);

                let mut data = Vec::new();
                crypto::compress(lua, &mut data).map_err(mlua::Error::external)?;
                crypto::encrypt_ulib(&mut data);
                let data = hex::encode_upper(&data);

                write!(s, r#"__U_Lib("{name}", "{data}")"#).unwrap();
                s.push('\n');
            }

            let mut bytecode = lua::compile(s)?;
            crypto::encrypt_res(&mut bytecode);
            Ok(bytecode)
        }

        pub fn set_database(&mut self, bytecode: Vec<u8>) {
            self.entries.insert("database.lua", bytecode);
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GameRes<'a, 'b> {
    keywords: Option<&'a str>,
    database: Option<&'b [u8]>,
    statistics: bool,
}

impl<'a, 'b> GameRes<'a, 'b> {
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
            hash = "",
            time = OffsetDateTime::now_utc().format(&time_fmt).unwrap(),
            keywords = self.keywords.unwrap_or_default()
        );

        let (b, _, _) = GBK.encode(&s);
        b.into_owned()
    }

    pub fn build(&self) -> Result<Vec<u8>, mlua::Error> {
        // compile database to bytecode
        let database = self.database.expect("database should set");
        let bytecode = lua::compile(database)?;

        // insert bundled library adaptor
        let adaptor = self.create_adaptor();

        // build bundles
        let mut bundles = Bundles::with_adaptor(lua::compile(adaptor)?);
        bundles.set_database(bytecode);
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
