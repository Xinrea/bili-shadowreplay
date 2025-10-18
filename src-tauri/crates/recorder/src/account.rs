#[derive(Debug, Clone)]
pub struct Account {
    pub platform: String,
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub csrf: String,
    pub cookies: String,
}
