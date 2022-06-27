use std::{
    error::Error as StdError,
    fmt::{self, Write},
    io,
};

use bstr::BString;
use encoding_rs::GBK;
use indexmap::IndexMap;
use mlua::Lua;
use time::OffsetDateTime;

#[derive(Debug)]
pub enum GameResError {
    IoError(io::Error),
    LuaVmError(mlua::Error),
}

impl fmt::Display for GameResError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameResError::IoError(err) => err.fmt(f),
            GameResError::LuaVmError(err) => err.fmt(f),
        }
    }
}

impl StdError for GameResError {}

#[derive(Debug, Clone, Default)]
pub struct GameRes {
    entries: IndexMap<String, Vec<u8>>,
}

impl GameRes {
    pub fn new() -> Self {
        GameRes {
            entries: IndexMap::new(),
        }
    }

    pub fn illegal_keywords(self, keywords: String) -> Self {
        self
    }

    pub fn anti_memory_cheat(self, switch: bool) -> Self {
        self
    }

    pub fn statistics(self, switch: bool) -> Self {
        self
    }

    pub fn anti_speed_hack(self, switch: bool) -> Self {
        self
    }

    pub fn build_time(self, time: OffsetDateTime) -> Self {
        self
    }

    pub fn game_lua(self, data: Vec<u8>) -> Self {
        self
    }

    pub fn build(&self) -> Result<Vec<u8>, GameResError> {
        todo!()
    }

    pub fn insert(&mut self, name: String, lua: Vec<u8>) {
        self.entries.insert(name, lua);
    }

    pub fn to_res(&self) -> Result<Vec<u8>, GameResError> {
        let mut bytecode = self.to_bytecode();
        crypto::encrypt_res(&mut bytecode);
        Ok(bytecode)
    }

    pub fn to_bytecode(&self) -> Vec<u8> {
        let s = self.to_string();

        let lua = unsafe { Lua::unsafe_new() };
        let f = lua.load(&s).into_function().unwrap();
        let data: BString = lua
            .load("return string.dump")
            .eval::<mlua::Function>()
            .unwrap()
            .call(f)
            .unwrap();
        data.into()
    }

    pub fn from_res(mut res: Vec<u8>) -> Self {
        res.truncate(res.len() - 10);
        crypto::decrypt_res(&mut res);
        Self::from_bytecode(res)
    }

    pub fn from_bytecode(chunk: Vec<u8>) -> Self {
        let mut entries = IndexMap::new();

        let lua = unsafe { Lua::unsafe_new() };
        lua.scope(|s| {
            let dummy = s.create_function_mut(|_, (name, data): (String, String)| {
                let name = hex::decode(name).map_err(mlua::Error::external)?;
                let (name, _, _) = GBK.decode(&name);

                let mut lua = Vec::new();
                let mut data = hex::decode(data).map_err(mlua::Error::external)?;
                crypto::decrypt_ulib(&mut data);
                crypto::decompress(&data, &mut lua);

                entries.insert(name.into_owned(), lua);
                Ok(())
            })?;
            lua.globals().set("__U_Lib", dummy)?;
            lua.load(&chunk).exec()
        })
        .unwrap();

        Self { entries }
    }
}

impl fmt::Display for GameRes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (name, lua) in &self.entries {
            let (name, _, _) = GBK.encode(name);
            let name = hex::encode_upper(name);

            let mut data = Vec::new();
            crypto::compress(lua, &mut data);
            crypto::encrypt_ulib(&mut data);
            let data = hex::encode_upper(&data);

            f.write_fmt(format_args!(r#"__U_Lib("{name}", "{data}")"#))?;
            f.write_char('\n')?;
        }

        Ok(())
    }
}

pub mod crypto {
    use flate2::bufread::{ZlibDecoder, ZlibEncoder};
    use flate2::Compression;
    use rc4::Rc4;
    use rc4::{consts::*, KeyInit, StreamCipher};
    use std::io::{self, Cursor, Read};

    const RESOURCE_KEY: &[u8] = b"_Npi_dest__cc_&%_23";
    const ULIB_KEY: &[u8] = b"&!!__kl_\xB2\xE2_I_0";

    pub fn encrypt_res(plain: &mut [u8]) {
        let mut rc4 = Rc4::<U19>::new(RESOURCE_KEY.into());
        rc4.apply_keystream(plain);
    }

    pub fn decrypt_res(cipher: &mut [u8]) {
        encrypt_res(cipher)
    }

    pub fn encrypt_ulib(plain: &mut [u8]) {
        let mut rc4 = Rc4::<U14>::new(ULIB_KEY.into());
        rc4.apply_keystream(plain);
    }

    pub fn decrypt_ulib(cipher: &mut [u8]) {
        encrypt_ulib(cipher)
    }

    const COMPRESS_MAGIC: u32 = 0x033E0F0D;

    pub fn decompress(data: &[u8], buf: &mut Vec<u8>) -> Result<(), io::Error> {
        let mut cursor = Cursor::new(data);

        buf.resize(8, 0);
        cursor.read_exact(buf).unwrap();

        let magic = u32::from_le_bytes(buf[..4].try_into().unwrap());
        if magic != COMPRESS_MAGIC {
            panic!("");
        }

        let size = u32::from_le_bytes(buf[4..].try_into().unwrap());
        buf.resize(size.try_into().unwrap(), 0);

        ZlibDecoder::new(cursor).read_exact(buf)
    }

    pub fn compress(data: &[u8], buf: &mut Vec<u8>) -> Result<usize, io::Error> {
        buf.extend(COMPRESS_MAGIC.to_le_bytes());
        buf.extend(data.len().to_le_bytes());
        ZlibEncoder::new(data, Compression::default()).read_to_end(buf)
    }
}
