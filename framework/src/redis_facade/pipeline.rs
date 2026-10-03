//! The commands a pipeline or a transaction queues.

/// The commands [`pipeline`](super::RedisConnection::pipeline) or
/// [`transaction`](super::RedisConnection::transaction) send. Each method
/// queues one command; its reply comes back in the same place in the list
/// the call returns.
#[derive(Debug, Default)]
pub struct RedisPipeline {
    pub(crate) commands: Vec<(String, Vec<Vec<u8>>)>,
}

impl RedisPipeline {
    /// Queue any command.
    pub fn command<A: AsRef<[u8]>>(&mut self, name: &str, args: &[A]) -> &mut Self {
        self.commands.push((
            name.to_ascii_uppercase(),
            args.iter().map(|arg| arg.as_ref().to_vec()).collect(),
        ));
        self
    }

    /// Queue `SET key value`.
    pub fn set(&mut self, key: &str, value: &str) -> &mut Self {
        self.command("SET", &[key, value])
    }

    /// Queue `SETEX key seconds value`.
    pub fn set_ex(&mut self, key: &str, value: &str, seconds: u64) -> &mut Self {
        self.command("SETEX", &[key, &seconds.to_string(), value])
    }

    /// Queue `GET key`.
    pub fn get(&mut self, key: &str) -> &mut Self {
        self.command("GET", &[key])
    }

    /// Queue `DEL key...`.
    pub fn del(&mut self, keys: &[&str]) -> &mut Self {
        self.command("DEL", keys)
    }

    /// Queue `INCRBY key by`.
    pub fn incr(&mut self, key: &str, by: i64) -> &mut Self {
        self.command("INCRBY", &[key, &by.to_string()])
    }

    /// Queue `DECRBY key by`.
    pub fn decr(&mut self, key: &str, by: i64) -> &mut Self {
        self.command("DECRBY", &[key, &by.to_string()])
    }

    /// Queue `EXPIRE key seconds`.
    pub fn expire(&mut self, key: &str, seconds: i64) -> &mut Self {
        self.command("EXPIRE", &[key, &seconds.to_string()])
    }

    /// Queue `HSET key field value`.
    pub fn hset(&mut self, key: &str, field: &str, value: &str) -> &mut Self {
        self.command("HSET", &[key, field, value])
    }

    /// Queue `LPUSH key value...`.
    pub fn lpush(&mut self, key: &str, values: &[&str]) -> &mut Self {
        self.command("LPUSH", &with_key(key, values))
    }

    /// Queue `RPUSH key value...`.
    pub fn rpush(&mut self, key: &str, values: &[&str]) -> &mut Self {
        self.command("RPUSH", &with_key(key, values))
    }

    /// Queue `SADD key member...`.
    pub fn sadd(&mut self, key: &str, members: &[&str]) -> &mut Self {
        self.command("SADD", &with_key(key, members))
    }

    /// Queue `ZADD key score member`.
    pub fn zadd(&mut self, key: &str, member: &str, score: f64) -> &mut Self {
        self.command("ZADD", &[key, &score.to_string(), member])
    }

    /// Queue `PUBLISH channel message`.
    pub fn publish(&mut self, channel: &str, message: &str) -> &mut Self {
        self.command("PUBLISH", &[channel, message])
    }
}

/// `key` followed by `rest`, as one argument list.
pub(crate) fn with_key<'a>(key: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut args = Vec::with_capacity(rest.len() + 1);
    args.push(key);
    args.extend_from_slice(rest);
    args
}
