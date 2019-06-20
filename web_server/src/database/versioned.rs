use bytes::Bytes;
use std::{
    fmt::Write,
    sync::Arc,
    ops::{RangeBounds, Bound},
};
use arrayvec::{ArrayVec, Array};
use super::{
    tools::{increment, slice_to_u32}
};

#[derive(Debug)]
pub enum VersionedError {
    Sled(sled::Error),
    WriteFmt(std::fmt::Error),
    VersionEmpty,
    VersionUtf(std::str::Utf8Error),
    VersionParse(std::num::ParseIntError),
    ValueParse,
    AccessDenied,
    CounterInvalid,
    UnexpectedOldValue,
}

//const MIN_U32: &str = "0000000000";
//const MAX_U32: &str = "4294967295";
const MIN_U32: &str = "00000000";
const MAX_U32: &str = "FFFFFFFF";

pub fn get_value<T, R: RangeBounds<u32>, F: Fn(&[u8])->Option<T> >(tree: &Arc<sled::Tree>, branch: &str, id: u32, leaf: &str, ver: R, parse: F) -> Result<Option<(u32, T)>, VersionedError> {
    let mut from = String::with_capacity(32);

    write!(from, "{}/{:08X}/{}/", branch, id, leaf).map_err(VersionedError::WriteFmt)?;
    let base_len = from.len();

    let mut to = from.clone();

    let lo = match ver.start_bound() {
        Bound::Included(inc) => {
            write!(to, "{:08X}", inc).map_err(VersionedError::WriteFmt)?;
            Bound::Included(to)
        },
        Bound::Excluded(exc) => {
            write!(to, "{:08X}", exc).map_err(VersionedError::WriteFmt)?;
            Bound::Excluded(to)
        },
        Bound::Unbounded => {
            to.push_str(MIN_U32);
            Bound::Included(to)
        }
    };

    let hi = match ver.end_bound() {
        Bound::Included(inc) => {
            write!(from, "{:08X}", inc).map_err(VersionedError::WriteFmt)?;
            Bound::Included(from)
        },
        Bound::Excluded(exc) => {
            write!(from, "{:08X}", exc).map_err(VersionedError::WriteFmt)?;
            Bound::Excluded(from)
        },
        Bound::Unbounded => {
            from.push_str(MAX_U32);
            Bound::Included(from)
        }
    };

    println!("from: {:?}, to: {:?}", &hi, &lo);

    for pair in tree.range((lo, hi)).rev() {
        let (full_key, value) = pair.map_err(VersionedError::Sled)?;
        println!("full_key: {:?}", std::str::from_utf8(&full_key));
        let key = full_key.get(base_len..).ok_or(VersionedError::VersionEmpty)?;
        let key = std::str::from_utf8(key).map_err(VersionedError::VersionUtf)?;
        let key = u32::from_str_radix(key, 16).map_err(VersionedError::VersionParse)?;
        if !ver.contains(&key) {
            eprintln!("Strange version: {:?}", key);
            continue;
        }
        let value = parse(value.as_ref()).ok_or(VersionedError::ValueParse)?;
        return Ok(Some((key, value)));
    }
    Ok(None)
}

pub fn update_value<V, F: Fn(Option<&[u8]>) -> Option<V>>(tree: &Arc<sled::Tree>, branch: &str, id: u32, leaf: &str, func: F) -> Result<Option<sled::IVec>, VersionedError>
    where sled::IVec: From<V>,
{
    let mut key = String::with_capacity(32);
    write!(key, "{}/{:08X}/{}", branch, id, leaf).map_err(VersionedError::WriteFmt)?;
    tree.update_and_fetch(key, func).map_err(VersionedError::Sled)
}

pub fn inc_counter(tree: &Arc<sled::Tree>, branch: &str, id: u32, leaf: &str) -> Result<u32, VersionedError> {
    update_value(tree, branch, id, leaf, increment).and_then(|opt| opt.and_then(|ivec| slice_to_u32(ivec.as_ref())).ok_or(VersionedError::CounterInvalid) )
}

pub fn set_value<V>(tree: &Arc<sled::Tree>, branch: &str, id: u32, leaf: &str, ver: u32, value: V) -> Result<Option<sled::IVec>, VersionedError>
    where sled::IVec: From<V>,
{
    let mut key = String::with_capacity(32);
    write!(key, "{}/{:08X}/{}/{:08X}", branch, id, leaf, ver).map_err(VersionedError::WriteFmt)?;

    tree.set(key, value).map_err(VersionedError::Sled)
}

pub fn new_version<'a, V, A: Array<Item=(&'a str, V)>>(tree: &Arc<sled::Tree>, branch: &str, id: u32, counter: &str, leaf_values: A) -> Result<u32, VersionedError>
    where sled::IVec: From<V>
{
    let ver = inc_counter(tree, branch, id, counter)?;
    let leaf_values = ArrayVec::from(leaf_values);
    for (leaf, value) in leaf_values {
        let old_value = set_value(tree, branch, id, leaf, ver, value)?;
        if old_value.is_some() {
            eprintln!("Unexpected old value: {}/{}/{}/{}", branch, id, leaf, ver);
            //return Err(VersionedError::UnexpectedOldValue)
        }
    }
    Ok(ver)
}
