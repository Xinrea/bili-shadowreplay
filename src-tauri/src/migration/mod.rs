pub mod migration_methods;

use sqlx::migrate::MigrationType;

#[derive(Debug)]
pub enum MigrationKind {
    Up,
    Down,
}

#[cfg(feature = "headless")]
#[derive(Debug)]
pub struct Migration {
    pub version: i64,
    pub description: &'static str,
    pub sql: &'static str,
    pub kind: MigrationKind,
}

impl From<MigrationKind> for MigrationType {
    fn from(kind: MigrationKind) -> Self {
        match kind {
            MigrationKind::Up => Self::ReversibleUp,
            MigrationKind::Down => Self::ReversibleDown,
        }
    }
}
