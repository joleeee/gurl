use argh::FromArgs;
use gurl::Agent;
use url::Url;

#[derive(FromArgs)]
/// gURL
struct Args {
    #[argh(positional)]
    url: Url,
}

fn main() {
    let args: Args = argh::from_env();

    let resp = Agent::get(args.url).unwrap().run().unwrap();

    eprintln!("status: {}", resp.status);
    eprintln!("meta: {}", resp.meta);

    let body = String::from_utf8(resp.body).unwrap();
    println!("{}", body);
}
