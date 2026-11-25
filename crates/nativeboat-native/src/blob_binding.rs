//! Native backing handles for Node 22's unchanged `internal/blob` source.
//! Blob bytes and readers are owned by native typed-array storage; no JS
//! engine or host blob implementation is involved.

use crate::array_builtin::{to_integer_or_infinity, to_length};
use crate::prelude::*;
use crate::typed_array_builtin;
use crate::{ErrorKind, Key, Object, Property, Realm, Slot, Symbol, Thrown, Value, ops};
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use core::cell::RefCell;

type Store = Rc<RefCell<BTreeMap<String, (Value, f64, Value)>>>;

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn reader_bytes(realm: &Realm, reader: &Value, key: &Symbol) -> Result<Vec<u8>, Thrown> {
    reader
        .as_object()
        .and_then(|object| object.private_property(key))
        .and_then(|property| match property.slot {
            Slot::Data { value, .. } => typed_array_builtin::view_bytes(&value),
            Slot::Accessor { .. } => None,
        })
        .ok_or_else(|| realm.throw(ErrorKind::Type, "Invalid Blob reader"))
}

fn reader_offset(realm: &mut Realm, reader: &Value, key: &Symbol) -> Result<usize, Thrown> {
    match reader
        .as_object()
        .and_then(|object| object.private_property(key))
    {
        Some(property) => match property.slot {
            Slot::Data { value, .. } => Ok(to_length(realm, &value)? as usize),
            Slot::Accessor { .. } => Ok(0),
        },
        None => Ok(0),
    }
}

fn handle(realm: &mut Realm, bytes: Vec<u8>, key: &Symbol) -> Object {
    let object = realm.object();
    object.set_private(
        key,
        Property::data(Value::Object(typed_array_builtin::new_bytes(
            realm, bytes, None,
        ))),
    );
    let reader_bytes_key = Symbol::private("nativeboat.blob.reader.bytes");
    let reader_offset_key = Symbol::private("nativeboat.blob.reader.offset");
    let handle_key = key.clone();
    let bytes_key = reader_bytes_key.clone();
    let offset_key = reader_offset_key.clone();
    let get_reader = realm.function("getReader", 0, false, move |realm, this, _| {
        let bytes = handle_bytes(realm, &this, &handle_key)?;
        let reader = realm.object();
        reader.set_private(
            &bytes_key,
            Property::data(Value::Object(typed_array_builtin::new_bytes(
                realm, bytes, None,
            ))),
        );
        reader.set_private(&offset_key, Property::data(Value::Number(0.0)));
        let bytes_key = bytes_key.clone();
        let offset_key = offset_key.clone();
        let pull = realm.function("pull", 1, false, move |realm, this, args| {
            let bytes = reader_bytes(realm, &this, &bytes_key)?;
            let offset = reader_offset(realm, &this, &offset_key)?;
            let callback = arg(args, 0);
            if offset >= bytes.len() {
                return ops::call(
                    realm,
                    &callback,
                    Value::Undefined,
                    &[Value::Number(0.0), Value::Undefined],
                );
            }
            let end = bytes.len();
            if let Some(reader) = this.as_object() {
                reader.set_private(&offset_key, Property::data(Value::Number(end as f64)));
            }
            let chunk = array_buffer(realm, bytes[offset..end].to_vec())?;
            ops::call(
                realm,
                &callback,
                Value::Undefined,
                &[Value::Number((end - offset) as f64), chunk],
            )
        });
        reader.define(Key::name("pull"), Property::data(Value::Object(pull)));
        Ok(Value::Object(reader))
    });
    object.define(
        Key::name("getReader"),
        Property::hidden(Value::Object(get_reader)),
    );
    let slice_key = key.clone();
    let slice = realm.function("slice", 2, false, move |realm, this, args| {
        let bytes = handle_bytes(realm, &this, &slice_key)?;
        let start = to_integer_or_infinity(realm, &arg(args, 0))?;
        let end = if matches!(args.get(1), None | Some(Value::Undefined)) {
            bytes.len() as f64
        } else {
            to_integer_or_infinity(realm, &arg(args, 1))?
        };
        let index = |position: f64| {
            if position < 0.0 {
                (bytes.len() as f64 + position).max(0.0) as usize
            } else {
                position.min(bytes.len() as f64) as usize
            }
        };
        let start = index(start);
        let end = index(end).max(start);
        Ok(Value::Object(handle(
            realm,
            bytes[start..end].to_vec(),
            &slice_key,
        )))
    });
    object.define(Key::name("slice"), Property::hidden(Value::Object(slice)));
    object
}

fn contents(value: &Value, key: &Symbol) -> Option<Vec<u8>> {
    let object = value.as_object()?;
    let property = object.private_property(key)?;
    match property.slot {
        Slot::Data { value, .. } => typed_array_builtin::view_bytes(&value),
        Slot::Accessor { .. } => None,
    }
}

fn handle_bytes(realm: &Realm, value: &Value, key: &Symbol) -> Result<Vec<u8>, Thrown> {
    contents(value, key).ok_or_else(|| realm.throw(ErrorKind::Type, "Invalid Blob handle"))
}

fn array_buffer(realm: &mut Realm, bytes: Vec<u8>) -> Result<Value, Thrown> {
    let array = Value::Object(typed_array_builtin::new_bytes(realm, bytes, None));
    ops::get_named(realm, &array, "buffer")
}

fn file_bytes(path: &Value) -> Option<Vec<u8>> {
    let Value::String(path) = path else {
        return None;
    };
    let mut units = path.units().to_vec();
    if units.contains(&0) {
        return None;
    }
    units.push(0);
    let stat = crate::fs_win::stat(&units, false).ok()?;
    let length = usize::try_from(stat.size).ok()?;
    let mut descriptors = crate::fs_win::Descriptors::default();
    let fd = descriptors.open(&units, 0, 0).ok()?;
    let bytes = {
        let handle = descriptors.handle(fd).ok()?;
        let mut bytes = alloc::vec![0; length];
        let mut buffers = [&mut bytes[..]];
        crate::fs_win::read(handle, &mut buffers, 0).ok()?;
        bytes
    };
    let _ = descriptors.close(fd);
    Some(bytes)
}

/// `internalBinding('blob')`, consumed by Node's unchanged `internal/blob`.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let handle_key = Symbol::private("nativeboat.blob.bytes");
    let store: Store = Rc::new(RefCell::new(BTreeMap::new()));

    let key = handle_key.clone();
    let create = realm.function("createBlob", 2, false, move |realm, _, args| {
        let sources = arg(args, 0);
        let length = ops::get_named(realm, &sources, "length")?;
        let length = to_length(realm, &length)? as usize;
        let mut bytes = Vec::new();
        for index in 0..length {
            let source = ops::get(realm, &sources, &Key::Index(index as u32))?;
            // Parts are byte views, or the handles of Blobs among the parts.
            let part = typed_array_builtin::view_bytes(&source)
                .or_else(|| contents(&source, &key))
                .ok_or_else(|| {
                    realm.throw(ErrorKind::Type, "Blob source must be an ArrayBufferView")
                })?;
            bytes.extend(part);
        }
        Ok(Value::Object(handle(realm, bytes, &key)))
    });
    exports.define(
        Key::name("createBlob"),
        Property::data(Value::Object(create)),
    );

    let path_key = handle_key.clone();
    let from_path = realm.function("createBlobFromFilePath", 1, false, move |realm, _, args| {
        let Some(bytes) = file_bytes(&arg(args, 0)) else {
            return Ok(Value::Undefined);
        };
        let length = bytes.len() as f64;
        let handle = Value::Object(handle(realm, bytes, &path_key));
        Ok(Value::Object(
            realm.array(vec![handle, Value::Number(length)]),
        ))
    });
    exports.define(
        Key::name("createBlobFromFilePath"),
        Property::data(Value::Object(from_path)),
    );

    let concat = realm.function("concat", 1, false, |realm, _, args| {
        let buffers = arg(args, 0);
        let length = ops::get_named(realm, &buffers, "length")?;
        let length = to_length(realm, &length)? as usize;
        let mut output = Vec::new();
        for index in 0..length {
            let buffer = ops::get(realm, &buffers, &Key::Index(index as u32))?;
            let bytes = typed_array_builtin::buffer_of(&buffer)
                .ok_or_else(|| realm.throw(ErrorKind::Type, "Blob parts must be ArrayBuffers"))?;
            output.extend(bytes.borrow().iter().copied());
        }
        array_buffer(realm, output)
    });
    exports.define(Key::name("concat"), Property::data(Value::Object(concat)));

    let get_store = store.clone();
    let get = realm.function("getDataObject", 1, false, move |realm, _, args| {
        let id = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let value = get_store.borrow().get(&id).map(|(handle, length, type_)| {
            Value::Object(realm.array(vec![handle.clone(), Value::Number(*length), type_.clone()]))
        });
        Ok(value.unwrap_or(Value::Undefined))
    });
    exports.define(
        Key::name("getDataObject"),
        Property::data(Value::Object(get)),
    );
    let store_data = store.clone();
    let store_key = handle_key;
    let store_fn = realm.function("storeDataObject", 4, false, move |realm, _, args| {
        let id = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let handle = arg(args, 1);
        handle_bytes(realm, &handle, &store_key)?;
        store_data.borrow_mut().insert(
            id,
            (handle, ops::to_number(realm, &arg(args, 2))?, arg(args, 3)),
        );
        Ok(Value::Undefined)
    });
    exports.define(
        Key::name("storeDataObject"),
        Property::data(Value::Object(store_fn)),
    );
    let revoke_store = store;
    let revoke = realm.function("revokeObjectURL", 1, false, move |realm, _, args| {
        let id = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        revoke_store.borrow_mut().remove(&id);
        Ok(Value::Undefined)
    });
    exports.define(
        Key::name("revokeObjectURL"),
        Property::data(Value::Object(revoke)),
    );
    exports
}

#[cfg(test)]
mod tests;
