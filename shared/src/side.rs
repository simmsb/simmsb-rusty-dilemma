use core::hash::Hash;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Eq, PartialEq, defmt::Format, Hash, Clone, Copy, Debug)]
#[repr(u8)]
pub enum KeyboardSide {
    Left,
    Right,
}