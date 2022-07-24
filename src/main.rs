use gurl::Agent;

fn main() {
    let resp = Agent::get("gemini://gemini.circumlunar.space/".try_into().unwrap())
        .unwrap()
        .run();

    eprintln!("status: {}", resp.status);
    eprintln!("meta: {}", resp.meta);

    let body = String::from_utf8(resp.body).unwrap();
    println!("{}", body);
}
