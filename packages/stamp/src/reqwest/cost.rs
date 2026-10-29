use reqwest::Request;
use subbit_l2_dto::CatalogIndex;

/// Computes the cost of a request. Implemented for any
/// `Fn(&Request) -> u64 + Send + Sync`, so a closure works directly.
pub trait Cost: Send + Sync {
    fn cost(&self, req: &Request) -> u64;
}

impl<F> Cost for F
where
    F: Fn(&Request) -> u64 + Send + Sync,
{
    fn cost(&self, req: &Request) -> u64 {
        self(req)
    }
}

/// Quote by the request's path against a `CatalogIndex`.
impl Cost for CatalogIndex {
    fn cost(&self, req: &Request) -> u64 {
        self.cost(req.url().path())
    }
}
