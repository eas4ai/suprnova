use std::collections::BTreeMap;
use suprnova::search::{
    AttributeDecl, AttributeType, AttributeValue, Filter, IndexDescriptor, IndexSettings,
    QueryText, SearchDocument,
};
fn descriptor() -> IndexDescriptor {
    IndexDescriptor {
        name: "query_tests",
        model_table: "items",
        key_column: "id",
        attributes: vec![
            AttributeDecl {
                name: "price".into(),
                ty: AttributeType::Float,
                searchable: None,
                filterable: true,
                sortable: true,
                facet: true,
            },
            AttributeDecl {
                name: "title".into(),
                ty: AttributeType::Text,
                searchable: None,
                filterable: true,
                sortable: false,
                facet: false,
            },
        ],
        embed_source: None,
        soft_delete: false,
        search_touches: vec![],
        settings: IndexSettings::default,
        scope: None,
        scope_fields: vec![],
    }
}
#[test]
fn grammar_preserves_boolean_precedence_and_converts_values() -> Result<(), suprnova::FrameworkError>
{
    let f = Filter::parse(
        "price >= 2 AND price 1 TO 3 OR title = 'special'",
        &descriptor(),
    )?;
    let d = SearchDocument {
        index: "query_tests".into(),
        key: "1".into(),
        source_version: 1,
        attributes: BTreeMap::from([("price".into(), AttributeValue::Float(2.5))]),
        embed_text: None,
        title: None,
    };
    assert!(f.matches(&d));
    Ok(())
}
#[test]
fn invalid_grammar_names_the_attribute() {
    for value in [
        "missing = 2",
        "price = 'not a number'",
        "title IN [1]",
        "price = 2 trailing",
    ] {
        assert!(Filter::parse(value, &descriptor()).is_err(), "{value}");
    }
}
#[test]
fn phrases_negatives_and_lone_punctuation_are_distinct() {
    let q = QueryText::parse("green \"red apple\" -rotten - \"");
    assert_eq!(
        q.phrases,
        vec![vec!["red".to_string(), "apple".to_string()]]
    );
    assert_eq!(q.negatives, vec!["rotten"]);
    assert_eq!(q.words, vec!["green", "-", "\""]);
}

#[test]
fn grammar_rejects_unbound_attributes_bad_values_and_incomplete_syntax() {
    let mut descriptor = descriptor();
    descriptor.attributes.push(AttributeDecl {
        name: "hidden".into(),
        ty: AttributeType::Text,
        searchable: None,
        filterable: false,
        sortable: false,
        facet: false,
    });
    for grammar in [
        "hidden = 'secret'",
        "missing = 2",
        "price = true",
        "price IN [1, 'bad']",
        "price 3 TO 1",
        "price NOT = 2",
        "price IS true",
        "price =",
        "price = 'unfinished",
        "price IN [1 2]",
        "(price = 1",
        "price ! 1",
        "_geoRadius(0, 0, 10)",
    ] {
        assert!(Filter::parse(grammar, &descriptor).is_err(), "{grammar}");
    }
    assert!(Filter::parse(&"(".repeat(65), &descriptor).is_err());
    assert!(Filter::parse(&" ".repeat(65537), &descriptor).is_err());
}
#[test]
fn grammar_converts_datetimes_and_supports_escaped_quoted_strings()
-> Result<(), suprnova::FrameworkError> {
    let mut descriptor = descriptor();
    descriptor.attributes.push(AttributeDecl {
        name: "created".into(),
        ty: AttributeType::DateTime,
        searchable: None,
        filterable: true,
        sortable: false,
        facet: false,
    });
    assert!(matches!(
        Filter::parse("created >= 2026-01-01T00:00:00Z", &descriptor)?,
        Filter::Gte(_, AttributeValue::DateTime(_))
    ));
    assert_eq!(
        Filter::parse("title = 'it\\'s fine'", &descriptor)?,
        Filter::Eq("title".into(), AttributeValue::Text("it's fine".into()))
    );
    Ok(())
}
#[test]
fn geo_grammar_requires_one_declared_geo_attribute() -> Result<(), suprnova::FrameworkError> {
    let mut descriptor = descriptor();
    let geo = AttributeDecl {
        name: "location".into(),
        ty: AttributeType::Geo,
        searchable: None,
        filterable: true,
        sortable: true,
        facet: false,
    };
    descriptor.attributes.push(geo.clone());
    assert!(
        matches!(Filter::parse("_geoRadius(10, -20, 1000)",&descriptor)?,Filter::GeoRadius(ref name,_,radius) if name=="location" && radius == 1000.0)
    );
    for grammar in [
        "_geoRadius(91, 0, 1)",
        "_geoRadius(0, 181, 1)",
        "_geoRadius(0, 0, -1)",
        "_geoRadius(0, 0, NaN)",
    ] {
        assert!(Filter::parse(grammar, &descriptor).is_err());
    }
    descriptor.attributes.push(AttributeDecl {
        name: "other_location".into(),
        ..geo
    });
    assert!(Filter::parse("_geoRadius(0, 0, 1)", &descriptor).is_err());
    Ok(())
}

#[test]
fn filter_grammar_rejects_nesting_beyond_the_ast_bound() {
    let grammar = format!("{}title = 'rust'{}", "NOT (".repeat(100), ")".repeat(100));
    assert!(
        Filter::parse(&grammar, &descriptor())
            .unwrap_err()
            .to_string()
            .contains("nesting exceeds 64")
    );
}

#[test]
fn grammar_and_typed_filters_share_the_64_level_bound() {
    let grammar = format!("{}title = 'rust'", "NOT ".repeat(64));
    Filter::parse(&grammar, &descriptor()).unwrap();
    assert!(Filter::parse(&format!("NOT {grammar}"), &descriptor()).is_err());
}

fn reads_title(_: &suprnova::Viewer) -> Filter {
    Filter::Eq("title".into(), AttributeValue::Text("open".into()))
}

#[test]
fn a_scope_must_name_and_read_only_its_scope_fields() {
    let mut index = descriptor();
    index.scope = Some(reads_title);
    let error = index.validate().unwrap_err().to_string();
    assert!(error.contains("scope_fields"), "{error}");
    index.scope_fields = vec!["price"];
    index.validate().unwrap();
    let error = suprnova::search::apply_scope(&index, &suprnova::Viewer::Guest, None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("'title'"), "{error}");
    let document = SearchDocument {
        index: "query_tests".into(),
        key: "1".into(),
        source_version: 1,
        attributes: BTreeMap::from([("title".into(), AttributeValue::Text("open".into()))]),
        embed_text: None,
        title: None,
    };
    assert!(!suprnova::search::evaluate_scope(
        &index,
        &suprnova::Viewer::Guest,
        &document
    ));
    index.scope_fields = vec!["title"];
    assert!(suprnova::search::apply_scope(&index, &suprnova::Viewer::Guest, None).is_ok());
    assert!(suprnova::search::evaluate_scope(
        &index,
        &suprnova::Viewer::Guest,
        &document
    ));
}
