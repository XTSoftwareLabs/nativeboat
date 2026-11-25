use super::*;

static TEST_ACCOUNT: [u16; 5] = ['a' as u16, 'c' as u16, 'c' as u16, 't' as u16, 0];
static TEST_PASSWORD: [u8; 5] = [b'p', 0xce, 0xb1, b's', b's'];
static mut TEST_CREDENTIALS: [Credential; 3] = [
    Credential {
        kind: CRED_TYPE_GENERIC,
        blob_size: TEST_PASSWORD.len() as u32,
        blob: TEST_PASSWORD.as_ptr() as *mut u8,
        username: TEST_ACCOUNT.as_ptr() as *mut u16,
        ..Credential::zeroed_for_test()
    },
    Credential {
        kind: CRED_TYPE_GENERIC,
        blob_size: TEST_PASSWORD.len() as u32,
        blob: TEST_PASSWORD.as_ptr() as *mut u8,
        username: core::ptr::null_mut(),
        ..Credential::zeroed_for_test()
    },
    Credential {
        kind: CRED_TYPE_GENERIC,
        blob_size: 0,
        blob: core::ptr::null_mut(),
        username: TEST_ACCOUNT.as_ptr() as *mut u16,
        ..Credential::zeroed_for_test()
    },
];
static mut TEST_CREDENTIAL_LIST: [CredentialPtr; 3] = [core::ptr::null_mut(); 3];

impl Credential {
    const fn zeroed_for_test() -> Credential {
        Credential {
            flags: 0,
            kind: 0,
            target_name: core::ptr::null_mut(),
            comment: core::ptr::null_mut(),
            last_written: FileTime { low: 0, high: 0 },
            blob_size: 0,
            blob: core::ptr::null_mut(),
            persist: 0,
            attribute_count: 0,
            attributes: core::ptr::null_mut(),
            target_alias: core::ptr::null_mut(),
            username: core::ptr::null_mut(),
        }
    }
}

unsafe extern "system" fn write_ok(_: *const Credential, _: u32) -> Bool {
    1
}

unsafe extern "system" fn write_failed(_: *const Credential, _: u32) -> Bool {
    0
}

unsafe extern "system" fn read_missing(
    _: *const u16,
    _: u32,
    _: u32,
    _: *mut CredentialPtr,
) -> Bool {
    0
}

unsafe extern "system" fn read_found(
    _: *const u16,
    _: u32,
    _: u32,
    credential: *mut CredentialPtr,
) -> Bool {
    // SAFETY: this points at a static test record whose nested byte buffers
    // live for the whole test process.
    unsafe { credential.write(core::ptr::addr_of_mut!(TEST_CREDENTIALS[0])) };
    1
}

unsafe extern "system" fn delete_missing(_: *const u16, _: u32, _: u32) -> Bool {
    0
}

unsafe extern "system" fn enumerate_missing(
    _: *const u16,
    _: u32,
    _: *mut u32,
    _: *mut *mut CredentialPtr,
) -> Bool {
    0
}

unsafe extern "system" fn enumerate_found(
    _: *const u16,
    _: u32,
    count: *mut u32,
    credentials: *mut *mut CredentialPtr,
) -> Bool {
    // SAFETY: the static test array remains writable for this call; all
    // credential records and their string/blob data have static lifetimes.
    unsafe {
        let list = core::ptr::addr_of_mut!(TEST_CREDENTIAL_LIST).cast::<CredentialPtr>();
        list.write(core::ptr::addr_of_mut!(TEST_CREDENTIALS[0]));
        list.add(1)
            .write(core::ptr::addr_of_mut!(TEST_CREDENTIALS[1]));
        list.add(2)
            .write(core::ptr::addr_of_mut!(TEST_CREDENTIALS[2]));
        count.write(3);
        credentials.write(list);
    }
    1
}

unsafe extern "system" fn delete_ok(_: *const u16, _: u32, _: u32) -> Bool {
    1
}

unsafe extern "system" fn free_noop(_: *const c_void) {}

unsafe extern "system" fn not_found_error() -> u32 {
    ERROR_NOT_FOUND
}

unsafe extern "system" fn access_denied_error() -> u32 {
    5
}

unsafe extern "system" fn get_procedure_missing(_: *mut c_void, _: *const u8) -> *const c_void {
    core::ptr::null()
}

unsafe extern "system" fn get_procedure_present(_: *mut c_void, _: *const u8) -> *const c_void {
    write_ok as *const () as *const c_void
}

unsafe extern "system" fn load_error() -> u32 {
    126
}

fn mock_api() -> Api {
    Api {
        write: write_ok,
        read: read_missing,
        delete: delete_missing,
        enumerate: enumerate_missing,
        free: free_noop,
        last_error: not_found_error,
    }
}

fn mock_found_api() -> Api {
    Api {
        read: read_found,
        delete: delete_ok,
        enumerate: enumerate_found,
        ..mock_api()
    }
}

fn input(operation: Operation) -> Input {
    Input {
        operation,
        service: "nativeboat-test-service".into(),
        account: "test-account".into(),
        password: "disposable".into(),
    }
}

#[test]
fn keytar_module_exports_the_five_async_credential_operations() {
    let mut realm = Realm::new();
    let module = module_with_api(&mut realm, Ok(mock_api()));
    let names = module
        .own_keys()
        .into_iter()
        .filter_map(|key| match key {
            Key::String(name) => Some(name.to_rust_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "setPassword",
            "getPassword",
            "deletePassword",
            "findPassword",
            "findCredentials",
        ]
    );
}

#[test]
fn keytar_lookups_return_promises_and_queue_store_access() {
    let mut realm = Realm::new();
    let module = module_with_api(&mut realm, Ok(mock_api()));
    let method = ops::get(
        &mut realm,
        &Value::Object(module),
        &Key::name("getPassword"),
    )
    .unwrap();
    let promise = ops::call(
        &mut realm,
        &method,
        Value::Undefined,
        &[Value::str("service"), Value::str("account")],
    )
    .unwrap();
    assert!(matches!(
        promise,
        Value::Object(object) if matches!(object.data().kind, crate::Kind::Promise(_))
    ));
}

#[test]
fn credential_operations_match_keytar_missing_and_success_results() {
    let api = mock_api();
    assert!(matches!(
        perform(api, input(Operation::SetPassword)).unwrap(),
        Output::Undefined
    ));
    assert!(matches!(
        perform(api, input(Operation::GetPassword)).unwrap(),
        Output::Password(None)
    ));
    assert!(matches!(
        perform(api, input(Operation::DeletePassword)).unwrap(),
        Output::Deleted(false)
    ));
    assert!(matches!(
        perform(api, input(Operation::FindPassword)).unwrap(),
        Output::Password(None)
    ));
    assert!(matches!(
        perform(api, input(Operation::FindCredentials)).unwrap(),
        Output::Credentials(rows) if rows.is_empty()
    ));

    let api = Api {
        last_error: access_denied_error,
        ..mock_api()
    };
    assert!(
        perform(api, input(Operation::GetPassword))
            .unwrap_err()
            .contains("error 5")
    );
    assert!(
        perform(api, input(Operation::FindPassword))
            .unwrap_err()
            .contains("error 5")
    );
}

#[test]
fn credential_reads_and_enumeration_copy_results_before_releasing_native_memory() {
    let api = mock_found_api();
    assert!(matches!(
        perform(api, input(Operation::GetPassword)).unwrap(),
        Output::Password(Some(password)) if password == "pαss"
    ));
    assert!(matches!(
        perform(api, input(Operation::FindPassword)).unwrap(),
        Output::Password(Some(password)) if password == "pαss"
    ));
    assert!(matches!(
        perform(api, input(Operation::FindCredentials)).unwrap(),
        Output::Credentials(credentials)
            if credentials == [("acct".into(), "pαss".into())]
    ));
    assert!(matches!(
        perform(api, input(Operation::DeletePassword)).unwrap(),
        Output::Deleted(true)
    ));

    let empty_password_api = Api {
        read: read_found_empty_password,
        ..mock_api()
    };
    assert!(matches!(
        perform(empty_password_api, input(Operation::GetPassword)).unwrap(),
        Output::Password(Some(password)) if password.is_empty()
    ));
}

unsafe extern "system" fn read_found_empty_password(
    _: *const u16,
    _: u32,
    _: u32,
    credential: *mut CredentialPtr,
) -> Bool {
    // SAFETY: the test credential and its empty blob are static.
    unsafe { credential.write(core::ptr::addr_of_mut!(TEST_CREDENTIALS[2])) };
    1
}

#[test]
fn production_module_resolves_credential_apis_without_accessing_the_store() {
    let mut realm = Realm::new();
    let module = module(&mut realm);
    assert_eq!(module.own_keys().len(), 5);
}

#[test]
fn credential_api_loader_checks_module_and_required_functions_without_store_access() {
    // SAFETY: the null module takes the resolver's early error path before any
    // function-pointer callback can use it.
    let missing_module =
        unsafe { resolve_api(core::ptr::null_mut(), get_procedure_missing, load_error) };
    assert!(matches!(missing_module, Err(126)));
    // SAFETY: the fake procedure callback ignores the non-null sentinel value.
    let missing_function =
        unsafe { resolve_api(core::ptr::dangling_mut(), get_procedure_missing, load_error) };
    assert!(matches!(missing_function, Err(127)));
    // SAFETY: the callback returns a non-null sentinel address, and the test
    // checks resolution only; it never invokes the resolved function pointers.
    let resolved =
        unsafe { resolve_api(core::ptr::dangling_mut(), get_procedure_present, load_error) };
    assert!(resolved.is_ok());
    assert!(api().is_ok());
}

#[test]
fn credential_outputs_convert_to_expected_native_values() {
    let realm = Realm::new();
    assert!(matches!(
        to_value(&realm, Output::Undefined),
        Value::Undefined
    ));
    assert!(matches!(
        to_value(&realm, Output::Password(None)),
        Value::Null
    ));
    assert!(
        matches!(to_value(&realm, Output::Password(Some("secret".into()))), Value::String(value) if value.to_rust_lossy() == "secret")
    );
    assert!(matches!(
        to_value(&realm, Output::Deleted(true)),
        Value::Boolean(true)
    ));
    let values = to_value(
        &realm,
        Output::Credentials(vec![("account".into(), "secret".into())]),
    );
    assert_eq!(values.as_object().unwrap().own_keys().len(), 2);
}

#[test]
fn unavailable_api_and_queue_failures_reject_keytar_promises() {
    let mut realm = Realm::new();
    let module = module_with_api(&mut realm, Err(126));
    let get_password = ops::get(
        &mut realm,
        &Value::Object(module),
        &Key::name("getPassword"),
    )
    .unwrap();
    let promise = ops::call(
        &mut realm,
        &get_password,
        Value::Undefined,
        &[Value::str("service"), Value::str("account")],
    )
    .unwrap();
    assert!(matches!(
        promise.as_object().unwrap().data().kind,
        crate::Kind::Promise(ref state)
            if matches!(state.borrow().state, crate::promise_builtin::State::Rejected(_))
    ));

    let mut realm = Realm::new();
    realm.event_loop.io_create = || Err(5);
    let module = module_with_api(&mut realm, Ok(mock_api()));
    let get_password = ops::get(
        &mut realm,
        &Value::Object(module),
        &Key::name("getPassword"),
    )
    .unwrap();
    let promise = ops::call(
        &mut realm,
        &get_password,
        Value::Undefined,
        &[Value::str("service"), Value::str("account")],
    )
    .unwrap();
    assert!(matches!(
        promise.as_object().unwrap().data().kind,
        crate::Kind::Promise(ref state)
            if matches!(state.borrow().state, crate::promise_builtin::State::Rejected(_))
    ));
}

#[test]
fn credential_write_errors_and_password_size_limits_are_reported() {
    let api = Api {
        write: write_failed,
        last_error: access_denied_error,
        ..mock_api()
    };
    assert_eq!(
        perform(api, input(Operation::SetPassword)).unwrap_err(),
        "Windows Credential Manager error 5"
    );
    assert_eq!(blob_size(u32::MAX as usize).unwrap(), u32::MAX);
    assert!(
        blob_size(u32::MAX as usize + 1)
            .unwrap_err()
            .contains("4 GiB")
    );
}

#[test]
fn utf16_credential_names_stop_at_embedded_nuls() {
    assert_eq!(wide("凭据\0ignored"), [0x51ed, 0x636e, 0]);
}
