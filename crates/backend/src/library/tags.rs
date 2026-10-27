//! Smart tags: tags whose members are whatever a saved Focus matches.
//!
//! A smart tag stores its [`FocusQuery`] in `user_tag.query_json` and holds
//! no `user_tag_item` rows of its own, so it follows the library as albums
//! are added, liked, played or tagged. Before a focus runs, every tag
//! criterion that names a smart tag gets that tag's query attached
//! ([`with_smart_tags`]), and the SQL builder in [`crate::library::focus`]
//! compiles it in place. A smart tag may use other tags, smart ones
//! included, up to [`MAX_SMART_TAG_DEPTH`] levels; a cycle simply matches
//! nothing past the first repeat.

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel::sqlite::SqliteConnection;

use crate::library::focus::{Criterion, FocusEntity, FocusQuery};

/// How many smart tags one profile may define.
pub const MAX_SMART_TAGS: i64 = 50;
/// How deeply smart tags may refer to other smart tags.
pub const MAX_SMART_TAG_DEPTH: usize = 3;

#[derive(QueryableByName)]
struct SmartRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    query_json: String,
}

/// Whether a focus can define a smart tag: the browsers whose entities a
/// tag can hold.
pub fn smart_entity_supported(entity: FocusEntity) -> bool {
    matches!(
        entity,
        FocusEntity::Albums | FocusEntity::Songs | FocusEntity::Artists
    )
}

/// The stored queries of a profile's smart tags, by tag id. Rows whose JSON
/// no longer parses are skipped rather than failing every focus.
pub fn smart_tag_queries(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<HashMap<i64, FocusQuery>, diesel::result::Error> {
    Ok(diesel::sql_query(
        "SELECT CAST(id AS BIGINT) AS id, query_json FROM user_tag \
         WHERE user_id = ? AND query_json IS NOT NULL ORDER BY id LIMIT ?",
    )
    .bind::<Integer, _>(user_id)
    .bind::<BigInt, _>(MAX_SMART_TAGS)
    .load::<SmartRow>(connection)?
    .into_iter()
    .filter_map(|row| {
        serde_json::from_str::<FocusQuery>(&row.query_json)
            .ok()
            .filter(|query| query.validate().is_ok() && smart_entity_supported(query.entity))
            .map(|query| (row.id, query))
    })
    .collect())
}

fn mentions_tags(query: &FocusQuery) -> bool {
    query
        .filters
        .iter()
        .map(|filter| &filter.criterion)
        .chain(query.scope.iter())
        .any(|criterion| matches!(criterion, Criterion::Tag { .. }))
}

fn resolve_criterion(
    criterion: &mut Criterion,
    smart: &HashMap<i64, FocusQuery>,
    path: &mut Vec<i64>,
) {
    let Criterion::Tag {
        tag_id,
        smart: slot,
        ..
    } = criterion
    else {
        return;
    };
    *slot = None;
    let Some(definition) = smart.get(tag_id) else {
        return;
    };
    if path.contains(tag_id) || path.len() >= MAX_SMART_TAG_DEPTH {
        return;
    }
    path.push(*tag_id);
    let mut definition = definition.clone();
    resolve_query(&mut definition, smart, path);
    path.pop();
    *slot = Some(Box::new(definition));
}

fn resolve_query(query: &mut FocusQuery, smart: &HashMap<i64, FocusQuery>, path: &mut Vec<i64>) {
    for filter in &mut query.filters {
        resolve_criterion(&mut filter.criterion, smart, path);
    }
    for criterion in &mut query.scope {
        resolve_criterion(criterion, smart, path);
    }
}

/// Attaches smart tag definitions to a query's tag criteria.
pub fn resolve_with(query: &FocusQuery, smart: &HashMap<i64, FocusQuery>) -> FocusQuery {
    let mut resolved = query.clone();
    resolve_query(&mut resolved, smart, &mut Vec::new());
    resolved
}

/// The query with each smart tag it names expanded into that tag's focus.
/// Queries that name no tag are returned unchanged without a lookup.
pub fn with_smart_tags(
    connection: &mut SqliteConnection,
    query: &FocusQuery,
    user_id: i32,
) -> Result<FocusQuery, diesel::result::Error> {
    if !mentions_tags(query) {
        return Ok(query.clone());
    }
    let smart = smart_tag_queries(connection, user_id)?;
    Ok(resolve_with(query, &smart))
}

/// Every smart tag of a profile with its focus resolved, for facet counts.
pub fn resolved_smart_tags(
    connection: &mut SqliteConnection,
    user_id: i32,
) -> Result<Vec<(i64, FocusQuery)>, diesel::result::Error> {
    let smart = smart_tag_queries(connection, user_id)?;
    let mut tags = smart
        .iter()
        .map(|(id, query)| {
            let mut resolved = query.clone();
            resolve_query(&mut resolved, &smart, &mut vec![*id]);
            (*id, resolved)
        })
        .collect::<Vec<_>>();
    tags.sort_by_key(|(id, _)| *id);
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::focus::FocusFilter;

    fn tag(id: i64) -> FocusFilter {
        FocusFilter {
            criterion: Criterion::Tag {
                tag_id: id,
                name: None,
                smart: None,
            },
            exclude: false,
        }
    }

    fn query(filters: Vec<FocusFilter>) -> FocusQuery {
        FocusQuery {
            entity: FocusEntity::Albums,
            filters,
            ..FocusQuery::default()
        }
    }

    fn smart_of(criterion: &Criterion) -> Option<&FocusQuery> {
        match criterion {
            Criterion::Tag { smart, .. } => smart.as_deref(),
            _ => None,
        }
    }

    #[test]
    fn smart_tags_resolve_recursively_and_stop_at_cycles() {
        let mut smart = HashMap::new();
        smart.insert(1, query(vec![tag(2)]));
        smart.insert(2, query(vec![tag(1), tag(9)]));
        let resolved = resolve_with(&query(vec![tag(1), tag(9)]), &smart);

        let first = smart_of(&resolved.filters[0].criterion).expect("tag 1 resolved");
        let second = smart_of(&first.filters[0].criterion).expect("tag 2 resolved");
        // Tag 2 points back at tag 1: the cycle is cut there.
        assert!(smart_of(&second.filters[0].criterion).is_none());
        // Tag 9 is a manual tag and stays a plain tag criterion.
        assert!(smart_of(&second.filters[1].criterion).is_none());
        assert!(smart_of(&resolved.filters[1].criterion).is_none());
    }

    #[test]
    fn resolution_is_depth_limited() {
        let mut smart = HashMap::new();
        for id in 1..=6 {
            smart.insert(id, query(vec![tag(id + 1)]));
        }
        let resolved = resolve_with(&query(vec![tag(1)]), &smart);
        let mut depth = 0;
        let mut current = smart_of(&resolved.filters[0].criterion);
        while let Some(next) = current {
            depth += 1;
            current = smart_of(&next.filters[0].criterion);
        }
        assert_eq!(depth, MAX_SMART_TAG_DEPTH);
    }

    #[test]
    fn smart_definitions_never_serialize() {
        let mut smart = HashMap::new();
        smart.insert(1, query(Vec::new()));
        let resolved = resolve_with(&query(vec![tag(1)]), &smart);
        let json = serde_json::to_value(&resolved).expect("json");
        assert_eq!(
            json["filters"][0],
            serde_json::json!({ "facet": "tag", "tag_id": 1, "exclude": false })
        );
    }
}
