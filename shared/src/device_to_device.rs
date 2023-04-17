use core::hash::Hash;
use serde::{Deserialize, Serialize};

use crate::host_to_device::HostToDeviceMsg;

#[derive(Serialize, Deserialize, Eq, PartialEq, defmt::Format, Hash, Clone, Debug)]
pub enum DeviceToDevice {
    Forwarded(HostToDeviceMsg),
}