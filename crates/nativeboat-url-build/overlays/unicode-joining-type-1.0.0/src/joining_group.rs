include!("generated/joining_group.rs"); // pre-generated from unchanged upstream tables

const MASK: usize = BLOCK_SIZE - 1;
const SHIFT: usize = MASK.count_ones() as usize;

/// Look up the joining group for a character.
///
/// ### Example
///
/// ```
/// use unicode_joining_type::{get_joining_group, JoiningGroup};
///
/// assert_eq!(get_joining_group('ھ'), JoiningGroup::KnottedHeh);
/// ```
pub fn get_joining_group(chr: char) -> JoiningGroup {
    let u = chr as u32;

    if u <= LAST_CODEPOINT {
        JOINING_GROUP_BLOCKS
            [JOINING_GROUP_BLOCK_OFFSETS[u as usize >> SHIFT] as usize + (u as usize & MASK)]
    } else {
        NoJoiningGroup
    }
}
