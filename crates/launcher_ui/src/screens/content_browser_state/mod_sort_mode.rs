use vertex_i18n::tr;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ModSortMode {
    Relevance,
    LastUpdated,
    Popularity,
}

impl ModSortMode {
    pub(crate) const ALL: [ModSortMode; 3] = [
        ModSortMode::Popularity,
        ModSortMode::Relevance,
        ModSortMode::LastUpdated,
    ];

    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            ModSortMode::Relevance => tr!("Relevance"),
            ModSortMode::LastUpdated => tr!("Last Update"),
            ModSortMode::Popularity => tr!("Popularity"),
        }
    }
}
