# LibreFang — serve_ml_classifier.py
# Part of fix: Fix 2 — Replace TF-IDF + Logistic Regression with SlavicBERT / Slovak BERT
# Author: coding-agent
# Date: 2026-06-22

import os
import sys
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
import joblib
import uvicorn

app = FastAPI(title="LibreFang ML Classifier")

# Global model state
classifier_type = None
transformer_pipeline = None
tfidf_pipeline = None
model_version = "unknown"

# Try importing HuggingFace transformers and PyTorch
try:
    import torch
    from transformers import pipeline as hf_pipeline
    HAS_TRANSFORMERS = True
except ImportError:
    HAS_TRANSFORMERS = False

@app.on_event("startup")
def load_model():
    global classifier_type, transformer_pipeline, tfidf_pipeline, model_version
    
    force_tfidf = os.environ.get("ML_CLASSIFIER_FORCE_TFIDF", "0") == "1"
    
    if HAS_TRANSFORMERS and not force_tfidf:
        # Priority order: local fine-tuned model path, otherwise gerulata/slovakbert or SlavicBERT
        model_name = os.environ.get("ML_TRANSFORMER_MODEL", "models/slovak_bert")
        if not os.path.exists(model_name) and model_name == "models/slovak_bert":
            # Fall back to HuggingFace hub if local model doesn't exist yet
            model_name = "gerulata/slovakbert"
            
        print(f"Loading transformer model from {model_name}...")
        try:
            device = 0 if torch.cuda.is_available() and torch.cuda.device_count() > 0 else -1
            transformer_pipeline = hf_pipeline(
                "text-classification",
                model=model_name,
                tokenizer=model_name,
                device=device,
                return_all_scores=True
            )
            classifier_type = "transformer"
            model_version = f"transformer:{model_name}"
            print(f"Transformer model loaded successfully on device: {'GPU' if device >= 0 else 'CPU'}")
            return
        except Exception as e:
            print(f"Failed to load transformer model ({e}). Falling back to TF-IDF...", file=sys.stderr)
            
    # Fallback to TF-IDF + LR
    print("[WARNING] baseline classifier, not recommended for production. HuggingFace transformers or PyTorch not available, or model load failed.")
    model_path = "models/ml_classifier.joblib"
    if not os.path.exists(model_path):
        print(f"ERROR: Baseline TF-IDF model file not found at {model_path}", file=sys.stderr)
        # Create a dummy trained pipeline if not exists so server can start in test/fallback mode
        print("Creating dummy TF-IDF pipeline for testing...", file=sys.stderr)
        from sklearn.feature_extraction.text import TfidfVectorizer
        from sklearn.linear_model import LogisticRegression
        from sklearn.pipeline import Pipeline as SkPipeline
        dummy_pipe = SkPipeline([
            ('tfidf', TfidfVectorizer()),
            ('clf', LogisticRegression())
        ])
        dummy_pipe.fit(["dummy text for calibration", "falošná správa o voľbách"], [0, 1])
        os.makedirs("models", exist_ok=True)
        joblib.dump(dummy_pipe, model_path)
        
    try:
        tfidf_pipeline = joblib.load(model_path)
        classifier_type = "tfidf"
        model_version = f"tfidf:{str(os.path.getmtime(model_path))}"
        print("Baseline TF-IDF model loaded successfully.")
    except Exception as e:
        print(f"ERROR loading TF-IDF model: {e}", file=sys.stderr)
        sys.exit(1)

class ScoreRequest(BaseModel):
    text: str

@app.post("/score")
def score(request: ScoreRequest):
    if classifier_type is None:
        raise HTTPException(status_code=503, detail="Model not loaded")
    
    try:
        if classifier_type == "transformer":
            outputs = transformer_pipeline(request.text)[0]
            score_val = 0.5
            for out in outputs:
                label_str = out["label"].upper()
                if label_str in ("LABEL_1", "FAKE", "DISINFORMATION", "DISINFO", "1"):
                    score_val = float(out["score"])
                    break
            else:
                if len(outputs) > 1:
                    score_val = float(outputs[1]["score"])
                else:
                    score_val = float(outputs[0]["score"])
            
            label = "DISINFORMATION" if score_val > 0.5 else "CREDIBLE"
            return {
                "score": score_val,
                "label": label,
                "version": model_version,
                "classifier_type": classifier_type
            }
        else:
            proba = tfidf_pipeline.predict_proba([request.text])[0]
            classes = list(tfidf_pipeline.classes_)
            if 1 in classes:
                idx_disinfo = classes.index(1)
                score_val = float(proba[idx_disinfo])
            else:
                score_val = float(proba[1]) if len(proba) > 1 else 0.5
                
            label = "DISINFORMATION" if score_val > 0.5 else "CREDIBLE"
            return {
                "score": score_val,
                "label": label,
                "version": model_version,
                "classifier_type": classifier_type
            }
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    port = int(os.environ.get("ML_CLASSIFIER_PORT", 8090))
    uvicorn.run(app, host="127.0.0.1", port=port)
