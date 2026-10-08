//! One module per use case. Each exposes a struct holding its ports and an `execute` method.

pub mod authenticate;
pub mod consume_play_ticket;
pub mod create_character;
pub mod ensure_account;
pub mod get_character;
pub mod issue_play_ticket;
pub mod list_my_characters;
pub mod ping;

pub use authenticate::Authenticate;
pub use consume_play_ticket::{ConsumePlayTicket, TicketRejection};
pub use create_character::{CreateCharacter, CreateCharacterInput};
pub use ensure_account::EnsureAccount;
pub use get_character::GetCharacter;
pub use issue_play_ticket::{IssuePlayTicket, IssuePlayTicketInput};
pub use list_my_characters::ListMyCharacters;
pub use ping::{Ping, PingOutput};
