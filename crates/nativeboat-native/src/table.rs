//! Hash tables. The runtime links no Rust `std`, whose collections live
//! there, so it keeps its own open-addressing tables: linear probing over a
//! power-of-two slot array, hashed with a per-process random seed so that
//! crafted keys cannot predict collisions, and removal by backward shifting
//! instead of tombstones.

use alloc::vec::Vec;
use core::hash::{Hash, Hasher};
use core::sync::atomic::{AtomicU64, Ordering};

/// The process's hash seed; zero until first use.
static SEED: AtomicU64 = AtomicU64::new(0);

/// The process's random hash seed.
pub(crate) fn process_seed() -> u64 {
    let seed = SEED.load(Ordering::Relaxed);
    if seed != 0 {
        return seed;
    }
    let fresh = crate::platform::random() | 1;
    SEED.store(fresh, Ordering::Relaxed);
    fresh
}

/// A fast multiplicative word hasher. Slot indices use the product's high
/// bits, which every input bit reaches.
struct WordHasher(u64);

impl Hasher for WordHasher {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(word))
                .wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// A hash map from keys to values.
pub struct HashMap<K, V> {
    seed: u64,
    /// Occupied slots hold the key's hash, the key and the value.
    slots: Vec<Option<(u64, K, V)>>,
    /// `64 - log2(slots.len())`.
    shift: u32,
    len: usize,
}

impl<K: Hash + Eq, V> Default for HashMap<K, V> {
    fn default() -> Self {
        HashMap::with_seed(process_seed())
    }
}

impl<K: Hash + Eq, V> HashMap<K, V> {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty map hashing with `seed`.
    pub fn with_seed(seed: u64) -> Self {
        HashMap {
            seed,
            slots: Vec::new(),
            shift: 64,
            len: 0,
        }
    }

    fn hash(&self, key: &K) -> u64 {
        let mut hasher = WordHasher(self.seed);
        key.hash(&mut hasher);
        hasher.finish()
    }

    fn home(&self, hash: u64) -> usize {
        (hash >> self.shift) as usize
    }

    /// The slot holding `key`, or the empty slot where it belongs.
    fn find(&self, hash: u64, key: &K) -> Result<usize, usize> {
        let mask = self.slots.len() - 1;
        let mut index = self.home(hash);
        loop {
            match &self.slots[index] {
                None => return Err(index),
                Some((stored, candidate, _)) if *stored == hash && candidate == key => {
                    return Ok(index);
                }
                Some(_) => index = (index + 1) & mask,
            }
        }
    }

    /// Whether the map holds no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The value for `key`.
    pub fn get(&self, key: &K) -> Option<&V> {
        if self.len == 0 {
            return None;
        }
        let index = self.find(self.hash(key), key).ok()?;
        self.slots[index].as_ref().map(|(_, _, value)| value)
    }

    /// Sets the value for `key`, returning the previous one.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if (self.len + 1) * 4 > self.slots.len() * 3 {
            self.grow();
        }
        let hash = self.hash(&key);
        match self.find(hash, &key) {
            Ok(index) => {
                let (_, _, old) = self.slots[index]
                    .as_mut()
                    .expect("found slots are occupied");
                Some(core::mem::replace(old, value))
            }
            Err(index) => {
                self.slots[index] = Some((hash, key, value));
                self.len += 1;
                None
            }
        }
    }

    fn grow(&mut self) {
        let capacity = (self.slots.len() * 2).max(8);
        let old = core::mem::replace(&mut self.slots, (0..capacity).map(|_| None).collect());
        self.shift = 64 - capacity.trailing_zeros();
        let mask = capacity - 1;
        for (hash, key, value) in old.into_iter().flatten() {
            let mut index = self.home(hash);
            while self.slots[index].is_some() {
                index = (index + 1) & mask;
            }
            self.slots[index] = Some((hash, key, value));
        }
    }

    /// Removes `key`, returning its value.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if self.len == 0 {
            return None;
        }
        let mut hole = self.find(self.hash(key), key).ok()?;
        let (_, _, value) = self.slots[hole].take().expect("found slots are occupied");
        self.len -= 1;
        // Later entries of the probe run move back unless that would place
        // them before their home slot.
        let mask = self.slots.len() - 1;
        let mut next = (hole + 1) & mask;
        while let Some((hash, _, _)) = &self.slots[next] {
            let home = self.home(*hash);
            if (next.wrapping_sub(home) & mask) >= (next.wrapping_sub(hole) & mask) {
                self.slots[hole] = self.slots[next].take();
                hole = next;
            }
            next = (next + 1) & mask;
        }
        Some(value)
    }
}

impl<K: core::fmt::Debug, V: core::fmt::Debug> core::fmt::Debug for HashMap<K, V> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_map()
            .entries(
                self.slots
                    .iter()
                    .flatten()
                    .map(|(_, key, value)| (key, value)),
            )
            .finish()
    }
}

/// A set of keys.
pub struct HashSet<K>(HashMap<K, ()>);

impl<K: Hash + Eq> HashSet<K> {
    /// An empty set.
    pub fn new() -> Self {
        HashSet(HashMap::new())
    }

    /// Adds `key`, returning whether it was absent.
    pub fn insert(&mut self, key: K) -> bool {
        self.0.insert(key, ()).is_none()
    }
}

#[cfg(test)]
mod tests;
