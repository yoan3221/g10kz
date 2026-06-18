#!/usr/bin/env python3
"""Prompt Guard — OpenVINO iGPU inference (Llama-Prompt-Guard-2-22M)
Same /classify API as the old Rust pg-server.
"""

import os, math, logging, time
from contextlib import asynccontextmanager
import numpy as np
from fastapi import FastAPI
from pydantic import BaseModel
from tokenizers import Tokenizer
from openvino import Core

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
log = logging.getLogger("prompt-guard")

MODEL_PATH      = os.getenv("MODEL_PATH",      "/opt/prompt-guard/model-22m/model.onnx")
TOKENIZER_PATH  = os.getenv("TOKENIZER_PATH",  "/opt/prompt-guard/model-22m/tokenizer.json")
THRESHOLD       = float(os.getenv("INJECTION_THRESHOLD", "0.85"))
DEVICE          = os.getenv("OV_DEVICE",       "GPU.0")
CACHE_DIR       = os.getenv("OV_CACHE_DIR",    "/opt/prompt-guard/ov_cache")
MAX_SEQ_LEN     = 512

_tok       = None
_infer_req = None
_device_used = "none"

@asynccontextmanager
async def lifespan(app: FastAPI):
    global _tok, _infer_req, _device_used
    log.info(f"Loading tokenizer: {TOKENIZER_PATH}")
    _tok = Tokenizer.from_file(TOKENIZER_PATH)

    ie = Core()
    if CACHE_DIR:
        os.makedirs(CACHE_DIR, exist_ok=True)
        try:
            ie.set_property({"CACHE_DIR": CACHE_DIR})
        except Exception:
            pass

    avail = ie.available_devices
    log.info(f"OpenVINO devices: {avail}")

    target = DEVICE
    if target not in avail:
        log.warning(f"{target} not available, falling back to CPU")
        target = "CPU"

    log.info(f"Compiling model on {target}: {MODEL_PATH}")
    t0 = time.time()
    try:
        model = ie.read_model(MODEL_PATH)
        compiled = ie.compile_model(model, target)
        _device_used = target
    except Exception as e:
        log.warning(f"Compile on {target} failed: {e}. Trying CPU.")
        model = ie.read_model(MODEL_PATH)
        compiled = ie.compile_model(model, "CPU")
        _device_used = "CPU"

    log.info(f"Compiled in {(time.time()-t0):.1f}s on {_device_used}")
    _infer_req = compiled.create_infer_request()

    # warmup
    ids  = np.ones((1, 8), dtype=np.int64)
    mask = np.ones((1, 8), dtype=np.int64)
    _infer_req.infer({"input_ids": ids, "attention_mask": mask})
    log.info(f"Prompt Guard ready on {_device_used}, threshold={THRESHOLD}")
    yield

app = FastAPI(lifespan=lifespan)

class Req(BaseModel):
    text: str

class Resp(BaseModel):
    label: str
    score: float
    blocked: bool

@app.get("/health")
def health():
    return {"status": "ok", "device": _device_used}

@app.post("/classify", response_model=Resp)
def classify(req: Req) -> Resp:
    try:
        enc  = _tok.encode(req.text)
        ids  = np.array([enc.ids[:MAX_SEQ_LEN]],              dtype=np.int64)
        mask = np.array([enc.attention_mask[:MAX_SEQ_LEN]],   dtype=np.int64)
        _infer_req.infer({"input_ids": ids, "attention_mask": mask})
        logits = _infer_req.get_output_tensor(0).data[0]
        a, b = float(logits[0]), float(logits[1])
        if math.isnan(a) or math.isnan(b):
            log.warning("NaN logits — fail open")
            return Resp(label="BENIGN", score=0.0, blocked=False)
        m = max(a, b)
        e0, e1 = math.exp(a - m), math.exp(b - m)
        score   = e1 / (e0 + e1)
        blocked = score >= THRESHOLD
        label   = "MALICIOUS" if blocked else "BENIGN"
        return Resp(label=label, score=score, blocked=blocked)
    except Exception as e:
        log.warning(f"classify error: {e}")
        return Resp(label="BENIGN", score=0.0, blocked=False)

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8083, log_level="info")
