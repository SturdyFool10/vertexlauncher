#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppScreen {
    Home,
    Library,
    Discover,
    DiscoverDetail,
    ContentBrowser,
    Skins,
    Settings,
    Legal,
    Console,
    Instance,
}

impl AppScreen {
    pub const FIXED_NAV: [AppScreen; 7] = [
        AppScreen::Home,
        AppScreen::Library,
        AppScreen::Discover,
        AppScreen::Skins,
        AppScreen::Settings,
        AppScreen::Legal,
        AppScreen::Console,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AppScreen::Home => "Home",
            AppScreen::Library => "Library",
            AppScreen::Discover => "Discover",
            AppScreen::DiscoverDetail => "Discover",
            AppScreen::ContentBrowser => "Content Browser",
            AppScreen::Skins => "Skin Manager",
            AppScreen::Settings => "Settings",
            AppScreen::Legal => "Legal",
            AppScreen::Console => "Console",
            AppScreen::Instance => "Instance",
        }
    }
}
