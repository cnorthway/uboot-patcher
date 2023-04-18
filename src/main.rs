use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{anyhow, Result};

fn redundant_env_bytes_to_hashmap(bytes: &[u8]) -> Result<HashMap<String, String>> {
    let single_len = bytes.len() / 2;

    let crc_one = u32::from_le_bytes(bytes[0..4].try_into()?);
    let crc_two = u32::from_le_bytes(bytes[single_len..single_len + 4].try_into()?);
    let calc_crc_one = crc32fast::hash(&bytes[5..single_len]);
    let calc_crc_two = crc32fast::hash(&bytes[single_len + 5..]);

    if !(crc_one == crc_two && crc_one == calc_crc_one && crc_one == calc_crc_two) {
        return Err(anyhow!(
            "CRC Mismatch! stored: {:#x} {:#x} calc: {:#x} {:#x}",
            crc_one,
            crc_two,
            calc_crc_one,
            calc_crc_two
        ));
    }

    Ok(HashMap::from_iter(
        bytes[5..single_len]
            // split data by null bytes
            .split(|b| *b == 0u8)
            .map(|sl| std::str::from_utf8(sl).unwrap())
            // filter to strings longer than length 0
            .filter(|s| s.len() > 0)
            // split on =
            .map(|line| line.split_once("=").unwrap())
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
    ))
}

fn hashmap_to_redundant_env_bytes(hm: HashMap<String, String>, len: usize) -> Result<Vec<u8>> {
    // max space for one 'data' portion
    // half the length (redundant halves), minus u32 crc, minus u8 flag
    let max_data_len = (len / 2) - 5;
    let mut data_bytes: Vec<u8> = Vec::with_capacity(max_data_len);

    // convert to key=val c strings
    for (key, val) in hm {
        data_bytes.extend(key.bytes());
        data_bytes.extend("=".bytes());
        data_bytes.extend(val.bytes());
        data_bytes.push(0);
    }

    let usage = data_bytes.len();

    if usage > max_data_len {
        return Err(anyhow!(
            "not enough space for environment ({} > {})",
            data_bytes.len(),
            max_data_len
        ));
    }

    // pad to length
    data_bytes.extend(vec![0; max_data_len - usage]);

    let crc = crc32fast::hash(&data_bytes);

    let mut total_vec: Vec<u8> = Vec::with_capacity(len);

    // first half
    total_vec.extend(u32::to_le_bytes(crc));
    total_vec.push(0b1); // flag as active
    total_vec.extend(&data_bytes);
    // second half
    total_vec.extend(u32::to_le_bytes(crc));
    total_vec.push(0b0); // flag as backup
    total_vec.extend(&data_bytes);

    if total_vec.len() != len {
        panic!("environment is the wrong size!");
    }

    Ok(total_vec)
}

fn read_file(filename: &str, offset: usize, len: usize) -> Result<HashMap<String, String>> {
    let mut f = File::open(filename)?;
    let mut buf = vec![0; len];
    f.seek(SeekFrom::Start(offset as u64))?;
    f.read(&mut buf)?;
    redundant_env_bytes_to_hashmap(&buf)
}

fn patch_file(
    hm: HashMap<String, String>,
    filename: &str,
    offset: usize,
    len: usize,
) -> Result<()> {
    let mut f = OpenOptions::new().write(true).open(filename)?;
    f.seek(SeekFrom::Start(offset as u64))?;
    f.write_all(&hashmap_to_redundant_env_bytes(hm, len)?)?;
    Ok(())
}

fn main() {
    // values set for eero,cento SPI flash
    let offset = 0x210000;
    let len = 0x20000;

    let backup_file = "backup.img";
    let new_file = "new.img";

    // note: using a hashmap as backing means order will change.
    // this doesn't (shouldn't) matter to u-boot in any way

    let mut hm = read_file(backup_file, offset, len).unwrap();

    // set bootdelay to a non-zero value
    hm.insert("bootdelay".to_string(), 5.to_string());
    // if you wish to further modify the environment, here's where you'd do so

    println!("new environment:");
    println!("{:#?}", hm);

    // copy content of old file
    std::fs::copy(backup_file, new_file).unwrap();
    // overwrite region with updated content
    patch_file(hm, new_file, offset, len).unwrap();
}
