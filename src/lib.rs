use madsim::plugin::{node, simulator, Simulator};
use madsim::rand::GlobalRng;
use madsim::task::{NodeId, Spawner};
use madsim::time::TimeHandle;
use madsim::Config;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

mod file_contents;

#[cfg(madsim)]
struct FsSimulator {
    file_systems: Mutex<HashMap<NodeId, Filesystem>>,
}

pub struct File {}

pub struct FileContents {}

pub struct Filesystem {
    files: HashMap<String, Arc<File>>,
}

impl Filesystem {
    fn with_current<R>(f: impl FnOnce(&mut Filesystem)) -> R {
        let node = node();
        simulator::<FsSimulator>()
    }

    fn reset(&mut self) {}
}

macro_rules! define_open_options {
    ($($name:ident),*) => {
        pub struct OpenOptions{
            $($name:bool),*
        }

        impl OpenOptions{
            pub fn $name(&mut self,$name:bool)-> &mut Self{
                self.$name = $name;
                self
            }
        }
    };
}

define_open_options! {read,write,append,truncate,create,create_new}

impl OpenOptions {
    async fn open(&self, p: impl AsRef<Path>) -> Result<File, std::io::Error> {}
}

impl Simulator for FsSimulator {
    fn new(rand: &GlobalRng, time: &TimeHandle, config: &Config) -> Self
    where
        Self: Sized,
    {
        todo!()
    }

    fn create_node(&self, id: NodeId) {
        let mut file_systems = self.file_systems.lock().unwrap();
        file_systems.insert(
            id,
            Filesystem {
                files: Default::default(),
            },
        );
    }

    fn reset_node(&self, _id: NodeId) {
        todo!()
    }
}
