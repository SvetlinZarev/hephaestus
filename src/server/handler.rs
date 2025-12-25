
pub async fn health_check() -> &'static str {
    "OK"
}

pub async fn metrics() -> String{
    "".to_owned()
}