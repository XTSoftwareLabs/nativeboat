//! AES cipher modes on CNG's block function, streamed as OpenSSL's EVP
//! interface streams them for Node's `Cipheriv` and `Decipheriv`: ECB and
//! CBC with PKCS#7 padding, CTR with a 128-bit counter and GCM with
//! additional data and authentication tags.

use crate::crypto_platform::AesKey;
use alloc::vec::Vec;

/// A cipher mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Ecb,
    Cbc,
    Ctr,
    Gcm,
}

/// The ciphers Node names that the binding provides: `(name, key length,
/// mode)`.
pub(crate) const CIPHERS: [(&str, usize, Mode); 12] = [
    ("aes-128-cbc", 16, Mode::Cbc),
    ("aes-128-ctr", 16, Mode::Ctr),
    ("aes-128-ecb", 16, Mode::Ecb),
    ("aes-128-gcm", 16, Mode::Gcm),
    ("aes-192-cbc", 24, Mode::Cbc),
    ("aes-192-ctr", 24, Mode::Ctr),
    ("aes-192-ecb", 24, Mode::Ecb),
    ("aes-192-gcm", 24, Mode::Gcm),
    ("aes-256-cbc", 32, Mode::Cbc),
    ("aes-256-ctr", 32, Mode::Ctr),
    ("aes-256-ecb", 32, Mode::Ecb),
    ("aes-256-gcm", 32, Mode::Gcm),
];

/// The key length and mode of a cipher name, in any letter case, with
/// OpenSSL's aliases (`aes128` for `aes-128-cbc`, `id-aes128-gcm`).
pub(crate) fn lookup(name: &str) -> Option<(usize, Mode)> {
    let name = name.to_ascii_lowercase();
    let name = match name.as_str() {
        "aes128" => "aes-128-cbc",
        "aes192" => "aes-192-cbc",
        "aes256" => "aes-256-cbc",
        "id-aes128-gcm" => "aes-128-gcm",
        "id-aes192-gcm" => "aes-192-gcm",
        "id-aes256-gcm" => "aes-256-gcm",
        other => other,
    };
    CIPHERS
        .iter()
        .find(|(known, _, _)| *known == name)
        .map(|(_, key, mode)| (*key, *mode))
}

/// Why a cipher operation failed, as OpenSSL reports it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Failure {
    /// Input not a whole number of blocks without padding, or a
    /// ciphertext whose length is not a whole number of blocks.
    WrongFinalBlockLength,
    /// Padding that is not PKCS#7 after decryption.
    BadDecrypt,
    /// A GCM tag that is missing or does not match.
    Authentication,
    /// Additional data after the text started, or any call after `final`.
    State,
}

/// An AES cipher in one direction, fed in chunks.
pub(crate) struct Cipher {
    key: AesKey,
    mode: Mode,
    decrypt: bool,
    padding: bool,
    /// The CBC chaining block or the CTR/GCM counter block.
    block: [u8; 16],
    /// Input held for a whole block (ECB, CBC).
    pending: Vec<u8>,
    /// Unused key stream bytes of the current counter block (CTR, GCM).
    stream: [u8; 16],
    stream_used: usize,
    gcm: Option<Gcm>,
    finished: bool,
}

struct Gcm {
    h: u128,
    j0: [u8; 16],
    state: u128,
    /// Bytes of additional data or text not yet a whole GHASH block.
    partial: Vec<u8>,
    aad_length: u64,
    text_length: u64,
    text_started: bool,
    tag_length: usize,
    expected: Option<Vec<u8>>,
    tag: Option<Vec<u8>>,
}

impl Cipher {
    /// A cipher of `mode` with a key of the mode's length and an IV of its
    /// length (16 bytes, none for ECB, any non-empty length for GCM).
    pub(crate) fn new(
        mode: Mode,
        key: &[u8],
        iv: &[u8],
        decrypt: bool,
        tag_length: usize,
    ) -> Result<Self, i32> {
        let key = AesKey::new(key)?;
        let mut block = [0u8; 16];
        let mut gcm = None;
        match mode {
            Mode::Ecb => {}
            Mode::Cbc | Mode::Ctr => block.copy_from_slice(iv),
            Mode::Gcm => {
                let mut h = [0u8; 16];
                key.encrypt(&mut h);
                let h = u128::from_be_bytes(h);
                let j0 = if iv.len() == 12 {
                    let mut j0 = [0u8; 16];
                    j0[..12].copy_from_slice(iv);
                    j0[15] = 1;
                    j0
                } else {
                    let mut state = 0;
                    for chunk in iv.chunks(16) {
                        state = ghash_block(state, h, chunk);
                    }
                    let mut lengths = [0u8; 16];
                    lengths[8..].copy_from_slice(&((iv.len() as u64) * 8).to_be_bytes());
                    ghash_block(state, h, &lengths).to_be_bytes()
                };
                block = j0;
                increment32(&mut block);
                gcm = Some(Gcm {
                    h,
                    j0,
                    state: 0,
                    partial: Vec::new(),
                    aad_length: 0,
                    text_length: 0,
                    text_started: false,
                    tag_length,
                    expected: None,
                    tag: None,
                });
            }
        }
        Ok(Self {
            key,
            mode,
            decrypt,
            padding: true,
            block,
            pending: Vec::new(),
            stream: [0; 16],
            stream_used: 16,
            gcm,
            finished: false,
        })
    }

    /// Turns PKCS#7 padding on or off (ECB and CBC).
    pub(crate) fn set_padding(&mut self, padding: bool) -> Result<(), Failure> {
        if self.finished {
            return Err(Failure::State);
        }
        self.padding = padding;
        Ok(())
    }

    /// Adds GCM additional authenticated data, before any text.
    pub(crate) fn add_aad(&mut self, data: &[u8]) -> Result<(), Failure> {
        let Some(gcm) = self.gcm.as_mut() else {
            return Err(Failure::State);
        };
        if gcm.text_started || self.finished {
            return Err(Failure::State);
        }
        gcm.aad_length += data.len() as u64;
        gcm.absorb(data);
        Ok(())
    }

    /// Sets the tag a GCM decryption must end with.
    pub(crate) fn set_tag(&mut self, tag: &[u8]) -> Result<(), Failure> {
        match self.gcm.as_mut() {
            Some(gcm) if self.decrypt && !self.finished && gcm.expected.is_none() => {
                gcm.expected = Some(tag.to_vec());
                Ok(())
            }
            _ => Err(Failure::State),
        }
    }

    /// The tag of a finished GCM encryption.
    pub(crate) fn tag(&self) -> Option<&[u8]> {
        self.gcm.as_ref()?.tag.as_deref()
    }

    /// Ciphers a chunk: the output OpenSSL gives for it.
    pub(crate) fn update(&mut self, data: &[u8]) -> Result<Vec<u8>, Failure> {
        if self.finished {
            return Err(Failure::State);
        }
        match self.mode {
            Mode::Ecb | Mode::Cbc => Ok(self.blocks(data)),
            Mode::Ctr => Ok(self.counter(data, false)),
            Mode::Gcm => {
                let gcm = self.gcm.as_mut().expect("GCM state");
                if !gcm.text_started {
                    gcm.text_started = true;
                    gcm.flush();
                }
                gcm.text_length += data.len() as u64;
                if self.decrypt {
                    gcm.absorb(data);
                }
                let output = self.counter(data, true);
                if !self.decrypt {
                    self.gcm.as_mut().expect("GCM state").absorb(&output);
                }
                Ok(output)
            }
        }
    }

    /// Ends the text: the last output, with padding added or checked, or
    /// the GCM tag made or checked.
    pub(crate) fn finish(&mut self) -> Result<Vec<u8>, Failure> {
        if self.finished {
            return Err(Failure::State);
        }
        self.finished = true;
        match self.mode {
            Mode::Ctr => Ok(Vec::new()),
            Mode::Gcm => {
                let tag_block = {
                    let mut block = self.gcm.as_ref().expect("GCM state").j0;
                    self.key.encrypt(&mut block);
                    block
                };
                let gcm = self.gcm.as_mut().expect("GCM state");
                gcm.flush();
                let mut lengths = [0u8; 16];
                lengths[..8].copy_from_slice(&(gcm.aad_length * 8).to_be_bytes());
                lengths[8..].copy_from_slice(&(gcm.text_length * 8).to_be_bytes());
                gcm.state = ghash_block(gcm.state, gcm.h, &lengths);
                let full = (gcm.state ^ u128::from_be_bytes(tag_block)).to_be_bytes();
                if self.decrypt {
                    let expected = gcm.expected.as_ref().ok_or(Failure::Authentication)?;
                    if expected.is_empty()
                        || expected.len() > 16
                        || !constant_time_equal(expected, &full[..expected.len()])
                    {
                        return Err(Failure::Authentication);
                    }
                } else {
                    gcm.tag = Some(full[..gcm.tag_length].to_vec());
                }
                Ok(Vec::new())
            }
            Mode::Ecb | Mode::Cbc => {
                if !self.decrypt {
                    if self.padding {
                        let pad = 16 - self.pending.len();
                        self.pending.resize(16, pad as u8);
                    } else if !self.pending.is_empty() {
                        return Err(Failure::WrongFinalBlockLength);
                    }
                    let mut block = core::mem::take(&mut self.pending);
                    self.cipher_blocks(&mut block);
                    return Ok(block);
                }
                if !self.padding {
                    return if self.pending.is_empty() {
                        Ok(Vec::new())
                    } else {
                        Err(Failure::WrongFinalBlockLength)
                    };
                }
                if self.pending.len() != 16 {
                    return Err(Failure::WrongFinalBlockLength);
                }
                let mut block = core::mem::take(&mut self.pending);
                self.cipher_blocks(&mut block);
                let pad = block[15] as usize;
                if pad == 0
                    || pad > 16
                    || block[16 - pad..].iter().any(|&byte| byte as usize != pad)
                {
                    return Err(Failure::BadDecrypt);
                }
                block.truncate(16 - pad);
                Ok(block)
            }
        }
    }

    /// ECB and CBC: whole blocks of the pending and new input; a
    /// decryption with padding holds back its last whole block for
    /// `finish`, as OpenSSL does.
    fn blocks(&mut self, data: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(data);
        let mut whole = self.pending.len() / 16 * 16;
        if self.decrypt && self.padding && whole == self.pending.len() && whole > 0 {
            whole -= 16;
        }
        let rest = self.pending.split_off(whole);
        let mut output = core::mem::replace(&mut self.pending, rest);
        self.cipher_blocks(&mut output);
        output
    }

    fn cipher_blocks(&mut self, blocks: &mut [u8]) {
        match (self.mode, self.decrypt) {
            (Mode::Ecb, false) => self.key.encrypt(blocks),
            (Mode::Ecb, true) => self.key.decrypt(blocks),
            (Mode::Cbc, false) => {
                for block in blocks.chunks_exact_mut(16) {
                    for (byte, chain) in block.iter_mut().zip(self.block) {
                        *byte ^= chain;
                    }
                    self.key.encrypt(block);
                    self.block.copy_from_slice(block);
                }
            }
            (Mode::Cbc, true) => {
                let ciphertext = blocks.to_vec();
                self.key.decrypt(blocks);
                for (index, block) in blocks.chunks_exact_mut(16).enumerate() {
                    let chain = if index == 0 {
                        self.block
                    } else {
                        let mut chain = [0; 16];
                        chain.copy_from_slice(&ciphertext[(index - 1) * 16..index * 16]);
                        chain
                    };
                    for (byte, chain) in block.iter_mut().zip(chain) {
                        *byte ^= chain;
                    }
                }
                if let Some(last) = ciphertext.len().checked_sub(16) {
                    self.block.copy_from_slice(&ciphertext[last..]);
                }
            }
            _ => unreachable!("only block modes cipher whole blocks"),
        }
    }

    /// CTR and GCM: the input XORed with the key stream of successive
    /// counter blocks; GCM increments only the counter's last 32 bits.
    fn counter(&mut self, data: &[u8], gcm: bool) -> Vec<u8> {
        let mut output = data.to_vec();
        let mut index = 0;
        while index < output.len() {
            if self.stream_used == 16 {
                let remaining = output.len() - index;
                let count = remaining.div_ceil(16).min(4096);
                if remaining >= 16 {
                    // Whole blocks of key stream in one CNG call.
                    let whole = (remaining / 16).min(count);
                    let mut stream = Vec::with_capacity(whole * 16);
                    for _ in 0..whole {
                        stream.extend_from_slice(&self.block);
                        self.advance(gcm);
                    }
                    self.key.encrypt(&mut stream);
                    for (byte, key) in output[index..index + whole * 16].iter_mut().zip(stream) {
                        *byte ^= key;
                    }
                    index += whole * 16;
                    continue;
                }
                self.stream = self.block;
                self.key.encrypt(&mut self.stream);
                self.advance(gcm);
                self.stream_used = 0;
            }
            output[index] ^= self.stream[self.stream_used];
            self.stream_used += 1;
            index += 1;
        }
        output
    }

    fn advance(&mut self, gcm: bool) {
        if gcm {
            increment32(&mut self.block);
        } else {
            let value = u128::from_be_bytes(self.block).wrapping_add(1);
            self.block = value.to_be_bytes();
        }
    }
}

impl Gcm {
    /// Adds bytes to GHASH, a block at a time.
    fn absorb(&mut self, mut data: &[u8]) {
        if !self.partial.is_empty() {
            let take = (16 - self.partial.len()).min(data.len());
            self.partial.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.partial.len() < 16 {
                return;
            }
            self.state = ghash_block(self.state, self.h, &self.partial);
            self.partial.clear();
        }
        let mut chunks = data.chunks_exact(16);
        for chunk in &mut chunks {
            self.state = ghash_block(self.state, self.h, chunk);
        }
        self.partial.extend_from_slice(chunks.remainder());
    }

    /// Pads the last partial block of additional data or text with zeros.
    fn flush(&mut self) {
        if !self.partial.is_empty() {
            self.state = ghash_block(self.state, self.h, &self.partial);
            self.partial.clear();
        }
    }
}

/// One GHASH step: `(state ^ block) * h` in GF(2^128), with a short block
/// padded with zeros.
fn ghash_block(state: u128, h: u128, block: &[u8]) -> u128 {
    let mut padded = [0u8; 16];
    padded[..block.len()].copy_from_slice(block);
    multiply(state ^ u128::from_be_bytes(padded), h)
}

/// Multiplication in GCM's field (NIST SP 800-38D, algorithm 1).
fn multiply(x: u128, y: u128) -> u128 {
    let mut product = 0;
    let mut value = y;
    for bit in 0..128 {
        if (x >> (127 - bit)) & 1 == 1 {
            product ^= value;
        }
        let low = value & 1;
        value >>= 1;
        if low == 1 {
            value ^= 0xE1 << 120;
        }
    }
    product
}

fn increment32(block: &mut [u8; 16]) {
    let counter = u32::from_be_bytes([block[12], block[13], block[14], block[15]]).wrapping_add(1);
    block[12..].copy_from_slice(&counter.to_be_bytes());
}

/// Compares two equally long byte strings in time independent of where
/// they differ.
pub(crate) fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests;
