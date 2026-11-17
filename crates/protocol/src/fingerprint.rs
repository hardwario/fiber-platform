//! The schema tags: a structural hash of the messages of a link, worked out when the crate is
//! built, folded to the 16 bits every frame carries.
//!
//! postcard does not describe itself: nothing in its bytes says which schema made them, so a peer
//! built from another source would be read wrong without a word. The tag closes that gap with
//! nothing to change by hand: it comes from the schema itself, so any change to the bytes moves
//! it, and a receiver refuses a frame whose tag is not its own.
//!
//! Every message derives [`postcard_schema::Schema`]. [`schema_hash`] walks the tree that gives
//! with an FNV-1a 64 hash over an encoding that cannot be read two ways: one byte for each kind
//! (each integer width, `Option`, sequence, tuple, struct, enum ...), the number of children
//! before them, the children in their order. **Names are left out**, because postcard does not
//! write them: a rename keeps the tag; a field moved, added, removed or widened moves it. The
//! hash of a link covers each type byte with the message it carries. It is folded to 16 bits:
//! two schemas share a tag once in 65536, which is plenty against a stale build, and no defence
//! against a forgery, which it is not meant to be.
//!
//! The method is TOWER's (`tower-protocol`); the code and the tags are this crate's own, so they
//! do not move with the TOWER pin.

use postcard_schema::Schema;
use postcard_schema::schema::{DataModelType, DataModelVariant, NamedType, NamedValue, NamedVariant};

const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

// One byte for each kind of node.
const T_BOOL: u8 = 0x01;
const T_I8: u8 = 0x02;
const T_U8: u8 = 0x03;
const T_I16: u8 = 0x04;
const T_I32: u8 = 0x05;
const T_I64: u8 = 0x06;
const T_I128: u8 = 0x07;
const T_U16: u8 = 0x08;
const T_U32: u8 = 0x09;
const T_U64: u8 = 0x0a;
const T_U128: u8 = 0x0b;
const T_USIZE: u8 = 0x0c;
const T_ISIZE: u8 = 0x0d;
const T_F32: u8 = 0x0e;
const T_F64: u8 = 0x0f;
const T_CHAR: u8 = 0x10;
const T_STRING: u8 = 0x11;
const T_BYTE_ARRAY: u8 = 0x12;
const T_OPTION: u8 = 0x13;
const T_UNIT: u8 = 0x14;
const T_UNIT_STRUCT: u8 = 0x15;
const T_NEWTYPE_STRUCT: u8 = 0x16;
const T_SEQ: u8 = 0x17;
const T_TUPLE: u8 = 0x18;
const T_TUPLE_STRUCT: u8 = 0x19;
const T_MAP: u8 = 0x1a;
const T_STRUCT: u8 = 0x1b;
const T_ENUM: u8 = 0x1c;
const T_SCHEMA: u8 = 0x1d;
const V_UNIT: u8 = 0x21;
const V_NEWTYPE: u8 = 0x22;
const V_TUPLE: u8 = 0x23;
const V_STRUCT: u8 = 0x24;

/// Where a link's hash starts: one for each link, so that two links with the same messages
/// still have different tags.
pub const SOUTH_LINK: u8 = 0x53;

pub const fn feed(mut state: u64, bytes: &[u8]) -> u64 {
    let mut i = 0;
    while i < bytes.len() {
        state ^= bytes[i] as u64;
        state = state.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    state
}

const fn feed_u8(state: u64, byte: u8) -> u64 {
    feed(state, &[byte])
}

const fn feed_count(state: u64, count: usize) -> u64 {
    feed(state, &(count as u32).to_le_bytes())
}

const fn hash_types(state: u64, tag: u8, types: &[&NamedType]) -> u64 {
    let mut state = feed_count(feed_u8(state, tag), types.len());
    let mut i = 0;
    while i < types.len() {
        state = hash_type(state, types[i]);
        i += 1;
    }
    state
}

const fn hash_fields(state: u64, tag: u8, fields: &[&NamedValue]) -> u64 {
    let mut state = feed_count(feed_u8(state, tag), fields.len());
    let mut i = 0;
    while i < fields.len() {
        state = hash_type(state, fields[i].ty);
        i += 1;
    }
    state
}

const fn hash_variants(state: u64, variants: &[&NamedVariant]) -> u64 {
    let mut state = feed_count(feed_u8(state, T_ENUM), variants.len());
    let mut i = 0;
    while i < variants.len() {
        state = match variants[i].ty {
            DataModelVariant::UnitVariant => feed_u8(state, V_UNIT),
            DataModelVariant::NewtypeVariant(inner) => hash_type(feed_u8(state, V_NEWTYPE), inner),
            DataModelVariant::TupleVariant(types) => hash_types(state, V_TUPLE, types),
            DataModelVariant::StructVariant(fields) => hash_fields(state, V_STRUCT, fields),
        };
        i += 1;
    }
    state
}

const fn hash_type(state: u64, ty: &NamedType) -> u64 {
    match ty.ty {
        DataModelType::Bool => feed_u8(state, T_BOOL),
        DataModelType::I8 => feed_u8(state, T_I8),
        DataModelType::U8 => feed_u8(state, T_U8),
        DataModelType::I16 => feed_u8(state, T_I16),
        DataModelType::I32 => feed_u8(state, T_I32),
        DataModelType::I64 => feed_u8(state, T_I64),
        DataModelType::I128 => feed_u8(state, T_I128),
        DataModelType::U16 => feed_u8(state, T_U16),
        DataModelType::U32 => feed_u8(state, T_U32),
        DataModelType::U64 => feed_u8(state, T_U64),
        DataModelType::U128 => feed_u8(state, T_U128),
        DataModelType::Usize => feed_u8(state, T_USIZE),
        DataModelType::Isize => feed_u8(state, T_ISIZE),
        DataModelType::F32 => feed_u8(state, T_F32),
        DataModelType::F64 => feed_u8(state, T_F64),
        DataModelType::Char => feed_u8(state, T_CHAR),
        DataModelType::String => feed_u8(state, T_STRING),
        DataModelType::ByteArray => feed_u8(state, T_BYTE_ARRAY),
        DataModelType::Option(inner) => hash_type(feed_u8(state, T_OPTION), inner),
        DataModelType::Unit => feed_u8(state, T_UNIT),
        DataModelType::UnitStruct => feed_u8(state, T_UNIT_STRUCT),
        DataModelType::NewtypeStruct(inner) => hash_type(feed_u8(state, T_NEWTYPE_STRUCT), inner),
        DataModelType::Seq(inner) => hash_type(feed_u8(state, T_SEQ), inner),
        DataModelType::Tuple(types) => hash_types(state, T_TUPLE, types),
        DataModelType::TupleStruct(types) => hash_types(state, T_TUPLE_STRUCT, types),
        DataModelType::Map { key, val } => hash_type(hash_type(feed_u8(state, T_MAP), key), val),
        DataModelType::Struct(fields) => hash_fields(state, T_STRUCT, fields),
        DataModelType::Enum(variants) => hash_variants(state, variants),
        DataModelType::Schema => feed_u8(state, T_SCHEMA),
    }
}

/// Folds one type's schema into a running hash.
pub const fn hash_schema(state: u64, ty: &NamedType) -> u64 {
    hash_type(state, ty)
}

/// The structural hash of `T`'s schema alone.
pub const fn schema_hash<T: Schema + ?Sized>() -> u64 {
    hash_type(FNV_BASIS, T::SCHEMA)
}

/// The hash a link starts from.
pub const fn link_basis(link: u8) -> u64 {
    feed_u8(FNV_BASIS, link)
}

/// A 64-bit hash folded to the 16-bit tag: the xor of its four 16-bit words.
pub const fn fold16(hash: u64) -> u16 {
    (hash ^ (hash >> 16) ^ (hash >> 32) ^ (hash >> 48)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    use postcard_schema::Schema;

    #[derive(Schema)]
    #[allow(dead_code)]
    struct A {
        x: u8,
        y: u16,
    }

    #[derive(Schema)]
    #[allow(dead_code)]
    struct Renamed {
        a: u8,
        b: u16,
    }

    #[derive(Schema)]
    #[allow(dead_code)]
    struct Swapped {
        y: u16,
        x: u8,
    }

    #[derive(Schema)]
    #[allow(dead_code)]
    struct Wider {
        x: u8,
        y: u32,
    }

    #[test]
    fn names_do_not_count_and_shapes_do() {
        assert_eq!(schema_hash::<A>(), schema_hash::<Renamed>());
        assert_ne!(schema_hash::<A>(), schema_hash::<Swapped>());
        assert_ne!(schema_hash::<A>(), schema_hash::<Wider>());
    }
}
