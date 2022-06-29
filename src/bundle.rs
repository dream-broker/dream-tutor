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