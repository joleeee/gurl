#[derive(Debug, Clone)]
pub struct Response {
    pub status: u8, // 2 digits
    pub meta: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
pub enum ResponseError {
    BadResponse,
}

impl Response {
    pub fn from_raw(input: &[u8]) -> Result<Self, ResponseError> {
        let (status, rest) = input.split_at(2);
        let status: u8 = String::from_utf8(status.to_vec())
            .ok()
            .ok_or(ResponseError::BadResponse)?
            .parse()
            .ok()
            .ok_or(ResponseError::BadResponse)?;

        let rest = &rest[1..];

        let (meta, body) = {
            let pos = rest
                .windows(2)
                .position(|window| window == [b'\r', b'\n'])
                .ok_or(ResponseError::BadResponse)?;

            let (meta, body) = rest.split_at(pos);

            (meta, &body[2..])
        };

        let meta = String::from_utf8(meta.to_vec()).unwrap();
        let body = body.to_vec();

        Ok(Self { status, meta, body })
    }
}
