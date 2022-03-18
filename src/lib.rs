#![warn(missing_docs)]
#![deny(warnings, clippy::pedantic, clippy::nursery, unused_crate_dependencies)]

//! Asynchronous and synchronous Postgres protocol transaction retries.
//!
//! Providing a mechanism for serialized transactions retries for `Postgres`
//! compatible databases.

#[cfg(feature = "async")]
use {
    deadpool_postgres::Pool as Deadpool,
    std::{future::Future, pin::Pin},
    tokio_postgres::error::SqlState,
};

#[cfg(feature = "sync")]
use {
    r2d2_postgres::PostgresConnectionManager,
    tokio::task::block_in_place,
    tokio_postgres::{
        tls::{MakeTlsConnect, TlsConnect},
        Socket,
    },
};

/// `CockroachDB` defined savepoint name.
pub const COCKROACH_SAVEPOINT: &str = "cockroach_restart";

/// Library errors.
pub enum Error<E> {
    /// `deadpool`.
    #[cfg(feature = "async")]
    Pool(deadpool::managed::PoolError<tokio_postgres::Error>),
    /// `tokio_postgres`.
    #[cfg(feature = "async")]
    Postgres(tokio_postgres::Error),
    /// `r2d2`.
    #[cfg(feature = "sync")]
    R2d2(r2d2::Error),
    /// All other errors, e.g. returned from transaction closure.
    Other(E),
}

#[cfg(feature = "async")]
impl<E> From<deadpool::managed::PoolError<tokio_postgres::Error>> for Error<E> {
    #[inline]
    fn from(e: deadpool::managed::PoolError<tokio_postgres::Error>) -> Self {
        Self::Pool(e)
    }
}

#[cfg(feature = "async")]
impl<E> From<tokio_postgres::Error> for Error<E> {
    #[inline]
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Postgres(e)
    }
}

#[cfg(feature = "sync")]
impl<E> From<r2d2::Error> for Error<E> {
    #[inline]
    fn from(e: r2d2::Error) -> Self {
        Self::R2d2(e)
    }
}

/// `r2d2` connection pool.
#[cfg(feature = "sync")]
pub type Pool<T> = r2d2::Pool<PostgresConnectionManager<T>>;

/// Result for async transactions.
#[cfg(feature = "async")]
pub type AsyncResult<'a, T, I> = Pin<Box<dyn Future<Output = Result<T, I>> + Send + 'a>>;

/// Executes the closure which is retried for serialization failures.
///
/// # Errors
///
/// Will return `Err` if there are any database errors or if the retry closure
/// returns `Err`.
#[cfg(feature = "async")]
#[inline]
pub async fn tx<T, E, I, S, F>(pool: &Deadpool, savepoint: S, f: F) -> Result<T, Error<E>>
where
    I: Into<Error<E>>,
    S: AsRef<str>,
    for<'a> F: Fn(&'a tokio_postgres::Transaction<'a>) -> AsyncResult<'a, T, I>,
{
    let mut client = pool.get().await?;
    let mut tx = client.transaction().await?;
    let savepoint = savepoint.as_ref();
    let v = loop {
        match execute_fn(&mut tx, savepoint, &f).await {
            Err(Error::Postgres(e)) if e.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) => {}
            r => break r,
        }
    }?;

    tx.commit().await?;

    Ok(v)
}

#[cfg(feature = "async")]
#[inline]
async fn execute_fn<T, E, I, F>(
    tx: &mut tokio_postgres::Transaction<'_>,
    savepoint: &str,
    f: &F,
) -> Result<T, Error<E>>
where
    I: Into<Error<E>>,
    for<'a> F: Fn(&'a tokio_postgres::Transaction<'a>) -> AsyncResult<'a, T, I>,
{
    let mut sp = tx.savepoint(savepoint).await?;
    let v = f(&mut sp).await.map_err(Into::into)?;

    sp.commit().await?;

    Ok(v)
}

/// Executes the closure which is retried for serialization failures.
///
/// # Errors
///
/// Will return `Err` if there are any database errors or if the retry closure
/// returns `Err`.
#[cfg(feature = "sync")]
#[inline]
pub fn tx_sync<T, M, E, I, S, F>(pool: &Pool<M>, savepoint: S, mut f: F) -> Result<T, Error<E>>
where
    M: MakeTlsConnect<Socket> + Clone + 'static + Sync + Send,
    M::TlsConnect: Send,
    M::Stream: Send,
    <M::TlsConnect as TlsConnect<Socket>>::Future: Send,
    I: Into<Error<E>>,
    S: AsRef<str>,
    F: FnMut(&mut postgres::Transaction<'_>) -> Result<T, I>,
{
    block_in_place(|| {
        let mut con = pool.get()?;
        let mut tx = con.transaction()?;
        let savepoint = savepoint.as_ref();

        loop {
            let mut sp = tx.savepoint(savepoint)?;

            match f(&mut sp)
                .map_err(Into::into)
                .and_then(|t| sp.commit().map(|_| t).map_err(Error::from))
            {
                Err(Error::Postgres(e))
                    if e.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) => {}
                r => break r,
            }
        }
        .and_then(|t| tx.commit().map(|_| t).map_err(Error::from))
    })
}
