//! The integration tests of the simulator: a `qi` client node exercising the services over TCP
//! loopback, like `naoqi_driver2` and a NAO HAL would.

mod common;

mod audio;
mod memory;
mod misc;
mod motion;
mod speech;
mod startup_contract;
mod video;
