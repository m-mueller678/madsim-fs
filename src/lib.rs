extern crate core;

use madsim::plugin::Simulator;
use std::future::Future;
use std::io::Write;
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite};

#[cfg(madsim)]
#[path = "simulated.rs"]
mod fs;

#[cfg(not(madsim))]
#[path = "tokio.rs"]
mod fs;

pub use fs::*;

macro_rules! define_open_options {
    ($($name:ident),*) => {
        pub struct OpenOptions{
            $($name:bool),*
        }

        impl OpenOptions{
            $(pub fn $name(&mut self,$name:bool)-> &mut Self{
                self.$name = $name;
                self
            })*
        }
    };
}

define_open_options! {read,write,append,truncate,create,create_new}
