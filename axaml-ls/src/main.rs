mod backend;
mod index;
mod metadata;
mod parser;

use backend::Backend;
use index::Index;
use std::collections::HashMap;
use std::sync::Mutex;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let index = Index::load();
    let (stdin, stdout) = (tokio::io::stdin(), tokio::io::stdout());
    let (service, socket) = LspService::new(|client| Backend {
        client,
        index,
        docs: Mutex::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
