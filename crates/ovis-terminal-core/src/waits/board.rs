//! The state every registration and every feed reads under the table lock.

use super::facts::Facts;
use crate::screen::Screen;

#[derive(Debug)]
pub(super) struct Board {
    pub screen: Screen,
    pub facts: Facts,
}
