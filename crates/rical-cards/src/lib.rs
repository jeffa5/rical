//! Address books of vCards (RFC 6350, and 3.0 and 2.1 as found): a folder
//! of address books, each a folder of `.vcf` files, as vdirsyncer keeps
//! them. Loading and keeping up with changes, searching every field, and
//! editing (losslessly, keeping each card's version), importing cards from
//! other apps by UID, and finding and merging duplicates.

pub mod book;
pub mod dedupe;
pub mod edit;
pub mod import;
pub mod model;
pub mod search;
pub mod text;

pub use book::{AddressBook, Books, CardId};
pub use model::{Address, Card, Field, Name};
