include!("generated/joining_type.rs"); // pre-generated from unchanged upstream tables

const MASK: usize = BLOCK_SIZE - 1;
const SHIFT: usize = MASK.count_ones() as usize;

/// Look up the joining type for a character.
///
/// ### Example
///
/// ```
/// use unicode_joining_type::{get_joining_type, JoiningType};
///
/// assert_eq!(get_joining_type('ھ'), JoiningType::DualJoining);
/// ```
pub fn get_joining_type(chr: char) -> JoiningType {
    let u = chr as u32;

    if u <= LAST_CODEPOINT {
        JOINING_TYPE_BLOCKS
            [JOINING_TYPE_BLOCK_OFFSETS[u as usize >> SHIFT] as usize + (u as usize & MASK)]
    } else {
        NonJoining
    }
}
