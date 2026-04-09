use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuPresenceContext {
    Screen(AppScreen),
    Home(HomePresenceSection),
    Instance(InstancePresenceSection),
}
