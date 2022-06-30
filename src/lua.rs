use bstr::BString;
use mlua::Lua;

pub fn compile(name: impl AsRef<str>, chunk: impl AsRef<[u8]>) -> Result<Vec<u8>, mlua::Error> {
    let lua = unsafe { Lua::unsafe_new() };

    let f = lua
        .load(chunk.as_ref())
        .set_name(name)?
        .into_function()
        .unwrap();

    let data: BString = lua
        .load("return string.dump")
        .eval::<mlua::Function>()
        .unwrap()
        .call(f)?;
    Ok(data.into())
}
