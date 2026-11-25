// Expected values are Node 22.15's (OpenSSL 3) for the same inputs.
use super::*;

fn hex(bytes: &[u8]) -> alloc::string::String {
    use core::fmt::Write;
    let mut text = alloc::string::String::new();
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

fn run(cipher: &mut Cipher, input: &[u8]) -> Vec<u8> {
    let mut output = cipher.update(input).unwrap();
    output.extend(cipher.finish().unwrap());
    output
}

#[test]
fn block_modes_pad_and_chain_as_openssl_does() {
    let mut ecb = Cipher::new(Mode::Ecb, &[5; 24], &[], false, 16).unwrap();
    let ciphertext = run(&mut ecb, b"0123456789abcdef0123");
    assert_eq!(
        hex(&ciphertext),
        "ccd434a778723d8c3ea14e4b38e2ff312de5c4fbad1eec3e2dad0fe2106a1728"
    );
    let mut ecb = Cipher::new(Mode::Ecb, &[5; 24], &[], true, 16).unwrap();
    assert_eq!(run(&mut ecb, &ciphertext), b"0123456789abcdef0123");

    let mut cbc = Cipher::new(Mode::Cbc, &[6; 32], &[2; 16], false, 16).unwrap();
    let ciphertext = run(&mut cbc, b"0123456789abcdef");
    assert_eq!(
        hex(&ciphertext),
        "3d675b127f1bfcd051af65a0d31feb93880f02dffd62481ab392337f585b37b8"
    );
    let mut cbc = Cipher::new(Mode::Cbc, &[6; 32], &[2; 16], true, 16).unwrap();
    assert_eq!(run(&mut cbc, &ciphertext), b"0123456789abcdef");
}

#[test]
fn decryption_holds_back_the_last_whole_block_until_final() {
    let mut encrypt = Cipher::new(Mode::Cbc, &[1; 16], &[2; 16], false, 16).unwrap();
    let ciphertext = run(&mut encrypt, &[7; 40]);
    let mut decrypt = Cipher::new(Mode::Cbc, &[1; 16], &[2; 16], true, 16).unwrap();
    let lengths = [
        decrypt.update(&ciphertext[..16]).unwrap().len(),
        decrypt.update(&ciphertext[16..40]).unwrap().len(),
        decrypt.update(&ciphertext[40..]).unwrap().len(),
        decrypt.finish().unwrap().len(),
    ];
    assert_eq!(lengths, [0, 32, 0, 8]);
}

#[test]
fn counter_mode_carries_through_all_128_bits() {
    let mut ctr = Cipher::new(Mode::Ctr, &[1; 16], &[0xff; 16], false, 16).unwrap();
    // One byte, then the rest: the key stream continues within a block.
    let mut output = ctr.update(&[0; 1]).unwrap();
    output.extend(ctr.update(&[0; 39]).unwrap());
    output.extend(ctr.finish().unwrap());
    assert_eq!(
        hex(&output),
        "6ad77cdab9adb3bfed21ef94210e121eb6aeaffa752dc08b51639731761aed00e431c158177be7de"
    );
}

#[test]
fn gcm_authenticates_additional_data_and_any_iv_length() {
    let text = b"hello gcm world, 33 bytes long!!!";
    let mut gcm = Cipher::new(Mode::Gcm, &[1; 16], &[3; 13], false, 16).unwrap();
    gcm.add_aad(b"aad-data-").unwrap();
    gcm.add_aad(b"longer-than-16").unwrap();
    let ciphertext = run(&mut gcm, text);
    assert_eq!(
        hex(&ciphertext),
        "16e420a23194e3eb5c8f0d601b72e7abb756889cb5c57e8f79068bae94c63d40c4"
    );
    let tag = gcm.tag().unwrap().to_vec();
    assert_eq!(hex(&tag), "3cae634feb6d9445a8f5d7fd75801c07");

    let mut open = Cipher::new(Mode::Gcm, &[1; 16], &[3; 13], true, 16).unwrap();
    open.set_tag(&tag).unwrap();
    open.add_aad(b"aad-data-longer-than-16").unwrap();
    assert_eq!(run(&mut open, &ciphertext), text);

    let mut short = Cipher::new(Mode::Gcm, &[9; 32], &[4; 12], false, 8).unwrap();
    assert_eq!(hex(&run(&mut short, b"abc")), "86dbdb");
    assert_eq!(hex(short.tag().unwrap()), "91b47f9595e0c325");
}

#[test]
fn failures_are_the_ones_openssl_reports() {
    let mut no_padding = Cipher::new(Mode::Cbc, &[1; 16], &[2; 16], false, 16).unwrap();
    no_padding.set_padding(false).unwrap();
    no_padding.update(b"abc").unwrap();
    assert_eq!(no_padding.finish(), Err(Failure::WrongFinalBlockLength));

    let mut bad = Cipher::new(Mode::Cbc, &[1; 16], &[2; 16], true, 16).unwrap();
    bad.update(&[0; 16]).unwrap();
    assert_eq!(bad.finish(), Err(Failure::BadDecrypt));

    let mut short = Cipher::new(Mode::Cbc, &[1; 16], &[2; 16], true, 16).unwrap();
    short.update(&[0; 10]).unwrap();
    assert_eq!(short.finish(), Err(Failure::WrongFinalBlockLength));

    let mut unpadded = Cipher::new(Mode::Ecb, &[1; 16], &[], true, 16).unwrap();
    unpadded.set_padding(false).unwrap();
    assert_eq!(unpadded.update(&[0; 16]).unwrap().len(), 16);
    assert_eq!(unpadded.finish(), Ok(Vec::new()));

    let mut untagged = Cipher::new(Mode::Gcm, &[1; 16], &[2; 12], true, 16).unwrap();
    untagged.update(&[0; 10]).unwrap();
    assert_eq!(untagged.finish(), Err(Failure::Authentication));

    let mut wrong_tag = Cipher::new(Mode::Gcm, &[1; 16], &[2; 12], true, 16).unwrap();
    wrong_tag.set_tag(&[0; 16]).unwrap();
    assert_eq!(wrong_tag.set_tag(&[0; 16]), Err(Failure::State));
    wrong_tag.update(&[0; 10]).unwrap();
    assert_eq!(wrong_tag.finish(), Err(Failure::Authentication));

    let mut late = Cipher::new(Mode::Gcm, &[1; 16], &[2; 12], false, 16).unwrap();
    assert!(late.tag().is_none());
    late.update(b"x").unwrap();
    assert_eq!(late.add_aad(b"y"), Err(Failure::State));
    assert_eq!(late.set_tag(&[0; 16]), Err(Failure::State));

    let mut done = Cipher::new(Mode::Cbc, &[1; 16], &[2; 16], false, 16).unwrap();
    assert_eq!(done.add_aad(b"y"), Err(Failure::State));
    done.finish().unwrap();
    assert_eq!(done.update(b"a"), Err(Failure::State));
    assert_eq!(done.finish(), Err(Failure::State));
    assert_eq!(done.set_padding(true), Err(Failure::State));
}

#[test]
fn cipher_names_and_aliases_resolve_to_key_lengths_and_modes() {
    assert_eq!(lookup("AES-256-GCM"), Some((32, Mode::Gcm)));
    assert_eq!(lookup("aes128"), Some((16, Mode::Cbc)));
    assert_eq!(lookup("id-aes192-GCM"), Some((24, Mode::Gcm)));
    assert_eq!(lookup("aes-128-ofb"), None);
    assert!(constant_time_equal(b"abc", b"abc"));
    assert!(!constant_time_equal(b"abc", b"abd"));
    assert!(!constant_time_equal(b"abc", b"ab"));
}
