# PostgreSQL CockroachDB Async Transaction Retries

Asynchronous and synchronous Postgres protocol transaction retries.

Providing a mechanism for serialized transactions retries for PostgresSQL
compatible databases.

## Error Type Requirements

The closure over the transaction must return an error that implements
`Into<postgres_tx_retry::Error<E>>` and for serialization failure determination,
your error type should implement (depending on enabled features):

- `From<deadpool::managed::PoolError<tokio_postgres::Error>>`
- `From<tokio_postgres::Error>`
- `From<r2d2::Error>`

A basic error type might be:

```rust
pub enum MyError {
    Pool(deadpool::managed::PoolError<tokio_postgres::Error>),
    Postgres(tokio_postgres::Error),
    R2d2(r2d2::Error),
    Message(&'static str),
}

impl Into<postgres_tx_retry::Error<Self>> for MyError {
    fn into(self) -> postgres_tx_retry::Error<Self> {
        match self {
            Self::Pool(e) => postgres_tx_retry::Error::Pool(e),
            Self::Postgres(e) => postgres_tx_retry::Error::Postgres(e),
            Self::R2d2(e) => postgres_tx_retry::Error::R2d2(e),
            _ => postgres_tx_retry::Error::Other(self),
        }
    }
}

impl<E> From<postgres_tx_retry::Error<E>> for MyError
where
    E: Into<Self>,
{
    fn from(src: postgres_tx_retry::Error<E>) -> Self {
        match src {
            postgres_tx_retry::Error::Pool(e) => Self::Pool(e),
            postgres_tx_retry::Error::Postgres(e) => Self::Postgres(e),
            postgres_tx_retry::Error::R2d2(e) => Self::R2d2(e),
            postgres_tx_retry::Error::Other(e) => e.into(),
        }
    }
}
```

## Asynchronous Example

```rust
let values = Arc::new("London");
let population = tx(&pool, COCKROACH_SAVEPOINT, |tx| {
    let values = values.clone();

    Box::pin(async move {
        let name = values.as_ref();
        let row = tx
            .query_one(
                "SELECT population FROM city WHERE name = $1",
                &[&name],
            )
            .await?;

        Ok(row.get(0))
    })
})
.await?;
```

## Synchronous Example

```rust
let name = "London";
let population = tx_sync(&db, COCKROACH_SAVEPOINT, |tx| {
    let row = tx.query_one(
        "SELECT population FROM city WHERE name = $1",
        &[&name],
    )?;

    Ok(row.get(0))
})?;
```
