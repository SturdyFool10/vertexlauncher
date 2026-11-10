use vertex_i18n::tr;
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

    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            AppScreen::Home => tr!("Home"),
            AppScreen::Library => tr!("Library"),
            AppScreen::Discover => tr!("Discover"),
            AppScreen::DiscoverDetail => tr!("Discover"),
            AppScreen::ContentBrowser => tr!("Content Browser"),
            AppScreen::Skins => tr!("Skin Manager"),
            AppScreen::Settings => tr!("Settings"),
            AppScreen::Legal => tr!("Legal"),
            AppScreen::Console => tr!("Console"),
            AppScreen::Instance => tr!("Instance"),
        }
    }
}
