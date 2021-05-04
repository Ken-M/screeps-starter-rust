pub mod tower;

use log::*;
use screeps::{LookConstant, Part, Position, ResourceType, ReturnCode, RoomObjectProperties, find, look::CREEPS, pathfinder::SearchResults, prelude::*};
use screeps::constants::find::*;
use crate::util::*;

use stdweb::serde ;
