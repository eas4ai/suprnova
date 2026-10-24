//! Auto-commit queries work over Bolt where supported and HTTPS on current HydraDB.
use crate::FrameworkError;
use neo4rs::{BoltType, Graph, query};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, time::Duration};

pub(super) type Record = HashMap<String, Value>;

pub(super) enum Transport {
    Bolt(Graph),
    Http(HttpTransport),
}

pub(super) struct HttpTransport {
    client: reqwest::Client,
    endpoint: url::Url,
    namespace: String,
    cell: String,
    authorization: reqwest::header::HeaderValue,
}

fn error(message: impl std::fmt::Display) -> FrameworkError {
    FrameworkError::database(format!("HydraDB: {message}"))
}

#[derive(Deserialize)]
struct HttpValue {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    value: Value,
}

#[derive(Deserialize)]
struct Page {
    columns: Vec<String>,
    rows: Vec<Vec<HttpValue>>,
    next_cursor: Option<u64>,
}

fn parameter(value: Value) -> Result<BoltType, FrameworkError> {
    Ok(match value {
        Value::Null => return Err(error("null query parameter is not supported")),
        Value::Bool(v) => v.into(),
        Value::String(v) => v.into(),
        Value::Number(v) => v
            .as_i64()
            .ok_or_else(|| error("parameter exceeds signed integer range"))?
            .into(),
        Value::Array(v) => v
            .into_iter()
            .map(parameter)
            .collect::<Result<Vec<_>, _>>()?
            .into(),
        Value::Object(v) => v
            .into_iter()
            .map(|(k, v)| Ok((k, parameter(v)?)))
            .collect::<Result<HashMap<_, _>, FrameworkError>>()?
            .into(),
    })
}

impl Transport {
    pub(super) async fn connect(
        address: &str,
        username: &str,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let mut endpoint = url::Url::parse(address).map_err(|_| error("invalid query URL"))?;
        if matches!(endpoint.scheme(), "bolt" | "bolt+s") {
            let database = endpoint.path().trim_matches('/');
            let database = if database.is_empty() {
                "default"
            } else {
                database
            };
            let config = neo4rs::ConfigBuilder::default()
                .uri(address)
                .user(username)
                .password(password)
                .db(database)
                .build()
                .map_err(error)?;
            return Ok(Self::Bolt(Graph::connect(config).await.map_err(error)?));
        }
        if !endpoint.path().starts_with("/v1/graphs/")
            || !endpoint.path().ends_with("/query")
            || endpoint.fragment().is_some()
        {
            return Err(error("HTTPS URL must name /v1/graphs/<graph>/query"));
        }
        let mut cell = "cell-0".to_owned();
        for (key, value) in endpoint.query_pairs() {
            if key != "cell_id" || value.is_empty() {
                return Err(error("URL accepts only a nonempty cell_id query parameter"));
            }
            cell = value.into_owned();
        }
        endpoint.set_query(None);
        let namespace = if username.is_empty() {
            "default"
        } else {
            username
        }
        .to_owned();
        let mut authorization =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {password}"))
                .map_err(|_| error("invalid bearer credential"))?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| error("could not build HTTP client"))?;
        Ok(Self::Http(HttpTransport {
            client,
            endpoint,
            namespace,
            cell,
            authorization,
        }))
    }

    pub(super) async fn execute(
        &self,
        statement: &str,
        parameters: Value,
        columns: &[&str],
    ) -> Result<Vec<Record>, FrameworkError> {
        tokio::time::timeout(
            Duration::from_secs(30),
            self.execute_inner(statement, parameters, columns),
        )
        .await
        .map_err(|_| error("query timed out"))?
    }

    async fn execute_inner(
        &self,
        statement: &str,
        parameters: Value,
        columns: &[&str],
    ) -> Result<Vec<Record>, FrameworkError> {
        match self {
            Self::Bolt(graph) => execute_bolt(graph, statement, parameters, columns).await,
            Self::Http(http) => http.execute(statement, parameters, columns).await,
        }
    }
}

async fn execute_bolt(
    graph: &Graph,
    statement: &str,
    parameters: Value,
    columns: &[&str],
) -> Result<Vec<Record>, FrameworkError> {
    let mut q = query(statement);
    for (key, value) in parameters
        .as_object()
        .ok_or_else(|| error("parameters must be an object"))?
    {
        q = q.param(key, parameter(value.clone())?);
    }
    // Do not BEGIN: HydraDB only supports auto-commit RUN requests.
    let mut stream = graph.execute(q).await.map_err(error)?;
    let mut records = Vec::new();
    while let Some(row) = stream.next().await.map_err(error)? {
        let mut record = Record::new();
        for column in columns {
            let value = if *column == "confidence" {
                row.get::<Option<i64>>(column)
                    .map_err(error)?
                    .map_or(Value::Null, |v| json!(v))
            } else {
                row.get::<Option<String>>(column)
                    .map_err(error)?
                    .map_or(Value::Null, Value::String)
            };
            record.insert((*column).into(), value);
        }
        records.push(record);
    }
    Ok(records)
}

impl Page {
    fn decode(self, expected: &[&str]) -> Result<Vec<Record>, FrameworkError> {
        if self.columns.iter().map(String::as_str).collect::<Vec<_>>() != expected {
            return Err(error(
                "query response columns differ from the requested projection",
            ));
        }
        self.rows
            .into_iter()
            .map(|row| {
                if row.len() != self.columns.len() {
                    return Err(error("query response row has the wrong number of columns"));
                }
                self.columns
                    .iter()
                    .zip(row)
                    .map(|(column, value)| {
                        if !matches!(
                            value.kind.as_str(),
                            "string" | "null" | "integer" | "signed_integer"
                        ) {
                            return Err(error("query projection has an unsupported type"));
                        }
                        Ok((column.clone(), value.value))
                    })
                    .collect()
            })
            .collect()
    }
}

impl HttpTransport {
    async fn execute(
        &self,
        statement: &str,
        parameters: Value,
        columns: &[&str],
    ) -> Result<Vec<Record>, FrameworkError> {
        let mut records = Vec::new();
        let mut request = json!({"cell_id":self.cell, "query":statement, "parameters":parameters, "page_size":500, "cursor":null, "query_id":uuid::Uuid::new_v4().to_string(), "consistency":"strong", "timeout_ms":25000});
        loop {
            let page = self.page(&request).await?;
            let cursor = page.next_cursor;
            records.extend(page.decode(columns)?);
            let Some(next) = cursor else {
                return Ok(records);
            };
            if request["cursor"].as_u64() == Some(next) {
                return Err(error("query cursor did not advance"));
            }
            request["cursor"] = json!(next);
        }
    }

    async fn page(&self, request: &Value) -> Result<Page, FrameworkError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::AUTHORIZATION, self.authorization.clone())
            .header("X-Graph-Namespace", &self.namespace)
            .json(request)
            .send()
            .await
            .map_err(|e| error(e.without_url()))?;
        if !response.status().is_success() {
            return Err(error(format!(
                "query rejected with HTTP {}: {}",
                response.status(),
                request["query"].as_str().unwrap_or("native query")
            )));
        }
        response
            .json()
            .await
            .map_err(|_| error("invalid JSON query response"))
    }
}
