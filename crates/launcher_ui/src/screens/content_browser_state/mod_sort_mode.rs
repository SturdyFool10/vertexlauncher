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

    pub(crate) fn label(self) -> &'static str {
        match self {
            ModSortMode::Relevance => "Relevance",
            ModSortMode::LastUpdated => "Last Update",
            ModSortMode::Popularity => "Popularity",
        }
    }
}
