use axum::{extract::State, routing::{get, post}, Json, Router};
use ort::{ep, inputs, session::Session, value::Tensor};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokenizers::Tokenizer;
use tracing::{info, warn};

const MAX_SEQ_LEN: usize = 512;

#[derive(Deserialize)]
struct Req { text: String }

#[derive(Serialize)]
struct Resp { label: String, score: f32, blocked: bool }

struct AppState {
    session:   Mutex<Session>,
    tokenizer: Tokenizer,
    threshold: f32,
}

async fn health() -> &'static str { "ok" }

async fn classify(State(st): State<Arc<AppState>>, Json(req): Json<Req>) -> Json<Resp> {
    let benign = || Json(Resp { label: "BENIGN".into(), score: 0.0, blocked: false });

    let enc = match st.tokenizer.encode(req.text.as_str(), false) {
        Ok(e) => e,
        Err(e) => { warn!(error = %e, "tokenize error"); return benign(); }
    };
    let ids: Vec<i64>  = enc.get_ids().iter().take(MAX_SEQ_LEN).map(|&x| x as i64).collect();
    let mask: Vec<i64> = enc.get_attention_mask().iter().take(MAX_SEQ_LEN).map(|&x| x as i64).collect();
    let seq_len = ids.len();

    let ids_val = match Tensor::<i64>::from_array((vec![1i64, seq_len as i64], ids)) {
        Ok(v) => v,
        Err(e) => { warn!(error = %e, "ids tensor error"); return benign(); }
    };
    let mask_val = match Tensor::<i64>::from_array((vec![1i64, seq_len as i64], mask)) {
        Ok(v) => v,
        Err(e) => { warn!(error = %e, "mask tensor error"); return benign(); }
    };

    // Run inference inside the lock scope; extract all data before dropping sess.
    // SessionOutputs borrows from the Session/MutexGuard, so we must not let it escape.
    let (score, blocked, label) = {
        let mut sess = match st.session.lock() {
            Ok(s) => s,
            Err(e) => { warn!(error = %e, "session lock error"); return benign(); }
        };
        let result = match sess.run(inputs![
            "input_ids"      => ids_val,
            "attention_mask" => mask_val
        ]) {
            Ok(r) => r,
            Err(e) => { warn!(error = %e, "ort run error"); return benign(); }
        };

        // ort 2.0.0-rc.12: try_extract_tensor returns (&Shape, &[T])
        let (_, flat) = match result["logits"].try_extract_tensor::<f32>() {
            Ok(t) => t,
            Err(e) => { warn!(error = %e, "logits extract error"); return benign(); }
        };

        if flat.len() < 2 { return benign(); }
        let max = flat.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exp: Vec<f32> = flat.iter().map(|&x| (x - max).exp()).collect();
        let sum: f32 = exp.iter().sum();
        let s = exp[1] / sum;
        let b = s >= st.threshold;
        let l = if b { "MALICIOUS" } else { "BENIGN" }.to_string();
        (s, b, l)  // only owned data escapes the lock
    };

    Json(Resp { label, score, blocked })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env()
            .add_directive("prompt_guard=info".parse()?))
        .init();
    let model_path = std::env::var("MODEL_PATH")
        .unwrap_or_else(|_| "/opt/prompt-guard/model/model.quant.onnx".into());
    let tokenizer_path = std::env::var("TOKENIZER_PATH")
        .unwrap_or_else(|_| "/opt/prompt-guard/model/tokenizer.json".into());
    let threshold: f32 = std::env::var("INJECTION_THRESHOLD")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(0.85);

    info!("Loading tokenizer from {tokenizer_path}");
    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;

    info!(model = %model_path, "Loading ONNX model — CUDA EP");
    let session = Session::builder()
        .map_err(|e| anyhow::anyhow!("session builder: {e}"))?
        .with_execution_providers([ep::CUDA::default().build()])
        .map_err(|e| anyhow::anyhow!("execution providers: {e}"))?
        .commit_from_file(&model_path)
        .map_err(|e| anyhow::anyhow!("model load: {e}"))?;

    info!("Session ready on CUDA");
    let state = Arc::new(AppState { session: Mutex::new(session), tokenizer, threshold });
    let app = Router::new()
        .route("/health", get(health))
        .route("/classify", post(classify))
        .with_state(state);
    let addr = "0.0.0.0:8083";
    info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
