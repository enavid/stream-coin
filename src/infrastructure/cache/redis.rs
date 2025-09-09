use redis::Client;
use redis::RedisError;
use redis::aio::MultiplexedConnection;


pub async fn establish_redis_connection(url: &str) -> Result<MultiplexedConnection, RedisError> {
    let client = Client::open(url)?;
    let conn = client.get_multiplexed_async_connection().await?;
    Ok(conn)
}