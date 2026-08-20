use crate::cost::url_lookup::UrlLookup;
use reqwest::Request;

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

/// Convenience impl for the common case: cost by path lookup.
impl Cost for UrlLookup {
    fn cost(&self, req: &Request) -> u64 {
        self.lookup(req.url().path())
    }
}
