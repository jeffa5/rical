use rical_core::{
    AnyComponent,
    query::{self, ListQuery},
};

use crate::{
    format::{Formatable, OutputFormat},
    pretty::PrettyComponent,
};

impl Formatable for AnyComponent {
    fn keys(&self) -> Vec<String> {
        let mut keys = vec!["path".to_owned()];
        let names = self
            .all_props()
            .iter()
            .map(|p| p.name.to_string())
            .collect::<std::collections::BTreeSet<_>>();
        keys.extend(names);
        keys
    }
}

pub fn list(query: &ListQuery, format: &OutputFormat, raw: bool) -> anyhow::Result<()> {
    let listing = query::list(query);

    let s = if raw {
        format.render(&listing.components)?
    } else {
        let short_ids = listing.short_ids();

        format.render(
            &listing
                .components
                .into_iter()
                .map(|c| {
                    let short_id = c.get_uid().and_then(|u| short_ids.get(u).cloned());
                    PrettyComponent::new(c, short_id)
                })
                .collect::<Vec<_>>(),
        )?
    };
    println!("{s}");

    Ok(())
}
