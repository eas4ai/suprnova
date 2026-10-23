use serde_json::{Value, json};
use suprnova::{
    IncludeResolutionError, IncludeTree, IncludedSink, IntoJsonResource, LengthAwarePaginator,
    RelationshipValue, Resource,
};

struct MetaResource;
impl IntoJsonResource for MetaResource {
    fn resource_type() -> &'static str {
        "items"
    }
    fn resource_id(&self) -> String {
        "1".into()
    }
    fn resource_attributes(&self, _: Option<&[&str]>) -> Value {
        json!({"name":"one"})
    }
    fn resource_relationships(&self) -> Vec<(String, RelationshipValue)> {
        vec![]
    }
    fn resource_included(
        &self,
        _: &IncludeTree,
        _: &mut IncludedSink,
    ) -> Result<(), IncludeResolutionError> {
        Ok(())
    }
    fn resource_meta(&self) -> serde_json::Map<String, Value> {
        json!({"item":1}).as_object().unwrap().clone()
    }
    fn resource_top_level_meta(&self) -> serde_json::Map<String, Value> {
        json!({"first_item":1}).as_object().unwrap().clone()
    }
}

#[tokio::test]
async fn collection_meta_belongs_to_the_collection() {
    let response = Resource::collection(vec![MetaResource])
        .with_meta("collection", json!(2))
        .render()
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["meta"], json!({"collection":2}));
    assert_eq!(body["data"][0]["meta"], json!({"item":1}));
    let response = Resource::paginated(LengthAwarePaginator::new(vec![MetaResource], 1, 10, 1))
        .render()
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(response.body()).unwrap();
    assert!(body["meta"].get("first_item").is_none());
    let response = Resource::single(MetaResource).render().await.unwrap();
    let body: Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["meta"]["first_item"], 1);
}

#[derive(Debug, Clone, suprnova::Data, suprnova::Validate)]
#[json_resource("groups")]
struct GroupResource {
    id: i64,
    name: String,
    fields: suprnova::MergeValue<Value>,
}

#[tokio::test]
async fn merge_groups_obey_the_condition_and_sparse_fields() {
    for condition in [false, true] {
        let dto = GroupResource {
            id: 1,
            name: "one".into(),
            fields: suprnova::merge_when(
                condition,
                json!({"email":"one@example.com", "nil":null, "missing":suprnova::Maybe::<i32>::missing()}),
            ),
        };
        let response = Resource::single(dto.clone()).render().await.unwrap();
        let body: Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["data"]["attributes"].get("email").is_some(), condition);
        assert_eq!(body["data"]["attributes"].get("nil").is_some(), condition);
        assert!(body["data"]["attributes"].get("fields").is_none());
        assert!(body["data"]["attributes"].get("missing").is_none());
        let response = suprnova::scope_fieldset(
            suprnova::RequestFieldsetSet::from_query("fields[groups]=email"),
            async { Resource::single(dto).render().await },
        )
        .await
        .unwrap();
        let body: Value = serde_json::from_slice(response.body()).unwrap();
        assert!(body["data"]["attributes"].get("name").is_none());
        assert_eq!(body["data"]["attributes"].get("email").is_some(), condition);
    }
}

#[test]
fn a_merge_group_refuses_non_object_fields() {
    assert!(serde_json::to_value(suprnova::merge_when(true, 7)).is_err());
    assert!(serde_json::to_value(suprnova::merge_when(false, 7)).is_ok());
}

#[tokio::test]
async fn application_jsonapi_default_and_response_override() {
    suprnova::jsonapi_default(Some(suprnova::JsonApiInfo::new().with_version("1.1"))).unwrap();
    for response in [
        Resource::single(MetaResource),
        Resource::collection(vec![MetaResource]),
        Resource::collection(Vec::<MetaResource>::new()),
        Resource::paginated(LengthAwarePaginator::new(vec![MetaResource], 1, 10, 1)),
    ] {
        let response = response.render().await.unwrap();
        let body: Value = serde_json::from_slice(response.body()).unwrap();
        assert_eq!(body["jsonapi"]["version"], "1.1");
    }
    let response = Resource::single(MetaResource)
        .with_jsonapi(suprnova::JsonApiInfo::new().with_version("1.0"))
        .render()
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(response.body()).unwrap();
    assert_eq!(body["jsonapi"]["version"], "1.0");
    suprnova::jsonapi_default(None).unwrap();
    let response = Resource::single(MetaResource).render().await.unwrap();
    let body: Value = serde_json::from_slice(response.body()).unwrap();
    assert!(body.get("jsonapi").is_none());
}

#[test]
fn merge_groups_deserialize_for_data_resources_and_preserve_omission() {
    for condition in [true, false] {
        let group = suprnova::merge_when(condition, json!({"name":"one"}));
        let encoded = serde_json::to_value(&group).unwrap();
        let decoded: suprnova::MergeValue<Value> = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
    }
    assert!(serde_json::from_value::<suprnova::MergeValue<Value>>(json!(7)).is_err());
}
