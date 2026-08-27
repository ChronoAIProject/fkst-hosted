use std::io::{self, Read};
use std::net::TcpStream;

const MAX_HEADERS: usize = 128;

pub(crate) struct Header {
    pub(crate) name: String,
    pub(crate) value: Vec<u8>,
}

pub(crate) struct RequestHead {
    pub(crate) method: String,
    pub(crate) target: String,
    pub(crate) headers: Vec<Header>,
    pub(crate) content_length: Option<usize>,
    pub(crate) buffered_body: Vec<u8>,
}

pub(crate) struct RequestHeadError {
    method: Option<String>,
    target: Option<String>,
}

impl RequestHeadError {
    fn new(method: Option<&str>, target: Option<&str>) -> Self {
        Self {
            method: method.map(str::to_owned),
            target: target.map(str::to_owned),
        }
    }

    pub(crate) fn request_target(&self) -> Option<(&str, &str)> {
        Some((self.method.as_deref()?, self.target.as_deref()?))
    }
}

pub(crate) enum BodyError {
    Truncated,
    TrailingBytes,
}

pub(crate) fn read_request_head(
    stream: &mut TcpStream,
    maximum_header_bytes: usize,
) -> io::Result<Result<RequestHead, RequestHeadError>> {
    let mut received = Vec::new();
    loop {
        let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
        let mut parsed = httparse::Request::new(&mut headers);
        match parsed.parse(&received) {
            Ok(httparse::Status::Complete(body_start)) => {
                if body_start > maximum_header_bytes {
                    return Ok(Err(RequestHeadError::new(parsed.method, parsed.path)));
                }
                let method = parsed
                    .method
                    .expect("complete request has a method")
                    .to_owned();
                let target = parsed
                    .path
                    .expect("complete request has a target")
                    .to_owned();
                let headers = parsed
                    .headers
                    .iter()
                    .map(|header| Header {
                        name: header.name.to_owned(),
                        value: trim_optional_whitespace(header.value).to_vec(),
                    })
                    .collect::<Vec<_>>();
                let content_length = match parse_framing(&headers) {
                    Ok(content_length) => content_length,
                    Err(()) => {
                        return Ok(Err(RequestHeadError {
                            method: Some(method),
                            target: Some(target),
                        }))
                    }
                };
                return Ok(Ok(RequestHead {
                    method,
                    target,
                    headers,
                    content_length,
                    buffered_body: received[body_start..].to_vec(),
                }));
            }
            Ok(httparse::Status::Partial) => {
                if received.len() >= maximum_header_bytes {
                    return Ok(Err(RequestHeadError::new(parsed.method, parsed.path)));
                }
            }
            Err(_) => return Ok(Err(RequestHeadError::new(parsed.method, parsed.path))),
        }

        let mut chunk = [0_u8; 1024];
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Ok(Err(RequestHeadError::new(None, None)));
        }
        received.extend_from_slice(&chunk[..read]);
    }
}

fn trim_optional_whitespace(mut value: &[u8]) -> &[u8] {
    while value
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[1..];
    }
    while value
        .last()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        value = &value[..value.len() - 1];
    }
    value
}

fn parse_framing(headers: &[Header]) -> Result<Option<usize>, ()> {
    let mut content_length = None;
    for header in headers {
        if header.name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(());
        }
        if header.name.eq_ignore_ascii_case("content-length") {
            if content_length.is_some()
                || header.value.is_empty()
                || !header.value.iter().all(u8::is_ascii_digit)
            {
                return Err(());
            }
            let value = std::str::from_utf8(&header.value)
                .map_err(|_| ())?
                .parse::<usize>()
                .map_err(|_| ())?;
            content_length = Some(value);
        }
    }
    Ok(content_length)
}

pub(crate) fn read_body(
    stream: &mut TcpStream,
    mut body: Vec<u8>,
    content_length: usize,
) -> io::Result<Result<Vec<u8>, BodyError>> {
    if body.len() > content_length {
        return Ok(Err(BodyError::TrailingBytes));
    }
    if body.len() < content_length {
        let mut remaining = vec![0_u8; content_length - body.len()];
        if let Err(error) = stream.read_exact(&mut remaining) {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                return Ok(Err(BodyError::Truncated));
            }
            return Err(error);
        }
        body.extend_from_slice(&remaining);
    }
    Ok(Ok(body))
}
