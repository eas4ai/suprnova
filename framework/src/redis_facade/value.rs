//! A Redis reply, as `command`, `eval`, pipelines and transactions give it.

/// A reply from the server. Bulk strings stay bytes, since Redis values
/// need not be text; [`as_str`](Self::as_str) reads one as text.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RedisValue {
    /// No value: a missing key, an empty pop.
    Nil,
    /// An integer reply.
    Int(i64),
    /// A bulk string.
    Bytes(Vec<u8>),
    /// A status reply, such as `OK` or `PONG`.
    Status(String),
    /// An array, a set, or the replies of a transaction.
    Array(Vec<RedisValue>),
    /// A map, from a server speaking RESP3.
    Map(Vec<(RedisValue, RedisValue)>),
    /// A double, from a server speaking RESP3.
    Double(f64),
    /// A boolean, from a server speaking RESP3.
    Bool(bool),
}

impl RedisValue {
    /// The reply as text: a bulk string that is UTF-8, or a status.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            RedisValue::Bytes(bytes) => std::str::from_utf8(bytes).ok(),
            RedisValue::Status(text) => Some(text),
            _ => None,
        }
    }

    /// The reply as an integer.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            RedisValue::Int(number) => Some(*number),
            _ => None,
        }
    }

    /// Whether the reply is [`Nil`](Self::Nil).
    pub fn is_nil(&self) -> bool {
        matches!(self, RedisValue::Nil)
    }
}

impl From<redis::Value> for RedisValue {
    fn from(value: redis::Value) -> Self {
        match value {
            redis::Value::Nil => RedisValue::Nil,
            redis::Value::Int(number) => RedisValue::Int(number),
            redis::Value::BulkString(bytes) => RedisValue::Bytes(bytes),
            redis::Value::SimpleString(text) => RedisValue::Status(text),
            redis::Value::Okay => RedisValue::Status("OK".to_owned()),
            redis::Value::Array(items) | redis::Value::Set(items) => {
                RedisValue::Array(items.into_iter().map(Into::into).collect())
            }
            redis::Value::Push { data, .. } => {
                RedisValue::Array(data.into_iter().map(Into::into).collect())
            }
            redis::Value::Map(pairs) => RedisValue::Map(
                pairs
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            ),
            redis::Value::Attribute { data, .. } => (*data).into(),
            redis::Value::Double(number) => RedisValue::Double(number),
            redis::Value::Boolean(flag) => RedisValue::Bool(flag),
            redis::Value::VerbatimString { text, .. } => RedisValue::Bytes(text.into_bytes()),
            // A big number, or a server error the caller already turned
            // into an `Err`: as text.
            other => match redis::from_redis_value::<String>(other.clone()) {
                Ok(text) => RedisValue::Bytes(text.into_bytes()),
                Err(_) => RedisValue::Bytes(format!("{other:?}").into_bytes()),
            },
        }
    }
}
