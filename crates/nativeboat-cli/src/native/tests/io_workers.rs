#![no_std]
#![no_main]
extern crate alloc;
extern crate nativeboat_start;

use nativeboat_native::{ErrorKind, Object, Realm, Thrown};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

fn module(realm: &mut Realm) -> Result<Object, Thrown> {
    let main = unsafe { GetCurrentThreadId() };
    let path: alloc::vec::Vec<u16> = __STAT_PATH__.encode_utf16().chain([0]).collect();
    realm.submit_io(move || {
        (unsafe { GetCurrentThreadId() }, nativeboat_native::fs_win::stat(&path, false))
    }, move |realm, (worker, result)| {
        let stat = result.map_err(|code| realm.throw(ErrorKind::Error, &alloc::format!("stat {code}")))?;
        nativeboat_native::platform::write_stdout(&alloc::format!(
            "worker {} completion {} size {}\n",
            worker != main, unsafe { GetCurrentThreadId() } == main, stat.size
        ));
        Ok(())
    }).map_err(|code| realm.throw(ErrorKind::Error, &alloc::format!("submit {code}")))?;
    Ok(realm.object())
}

#[unsafe(no_mangle)]
pub fn nativeboat_main() -> i32 {
    Realm::new().run_main(module)
}
