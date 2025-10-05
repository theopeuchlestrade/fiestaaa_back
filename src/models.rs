use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct LoginPayload { pub email: String, pub password: String }

#[derive(Deserialize)]
pub struct RegisterPayload { pub email: String, pub password: String }

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

