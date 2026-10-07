//! One module per use case. Each exposes a struct holding its ports and an `execute` method.

pub mod create_character;
pub mod get_character;
pub mod ping;

pub use create_character::{CreateCharacter, CreateCharacterInput};
pub use get_character::GetCharacter;
pub use ping::{Ping, PingOutput};
