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
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{:X}", magic),
        ));
    }

    let size = u32::from_le_bytes(buf[4..].try_into().unwrap());
    buf.resize(size.try_into().unwrap(), 0);

    ZlibDecoder::new(cursor).read_exact(buf)
}

pub fn compress(data: &[u8], buf: &mut Vec<u8>) -> Result<usize, io::Error> {
    let len: u32 = data.len().try_into().unwrap();
    buf.extend(COMPRESS_MAGIC.to_le_bytes());
    buf.extend(len.to_le_bytes());
    ZlibEncoder::new(data, Compression::default()).read_to_end(buf)
}
