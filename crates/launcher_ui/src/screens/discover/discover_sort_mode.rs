#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverSortMode {
    #[default]
    Popularity,
    Relevance,
    LastUpdated,
}

impl DiscoverSortMode {
    pub(crate) const ALL: [Self; 3] = [Self::Popularity, Self::Relevance, Self::LastUpdated];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Popularity => "Popularity",
            Self::Relevance => "Relevance",
            Self::LastUpdated => "Last Updated",
        }
    }
}
