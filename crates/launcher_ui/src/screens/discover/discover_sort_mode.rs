use vertex_i18n::tr;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum DiscoverSortMode {
    #[default]
    Popularity,
    Relevance,
    LastUpdated,
}

impl DiscoverSortMode {
    pub(crate) const ALL: [Self; 3] = [Self::Popularity, Self::Relevance, Self::LastUpdated];

    pub(crate) fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Popularity => tr!("Popularity"),
            Self::Relevance => tr!("Relevance"),
            Self::LastUpdated => tr!("Last Updated"),
        }
    }
}
