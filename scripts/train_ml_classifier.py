# LibreFang — train_ml_classifier.py
# Part of fix: Fix 2 — Replace TF-IDF + Logistic Regression with SlavicBERT / Slovak BERT
# Author: coding-agent
# Date: 2026-06-22

import argparse
import json
import os
import sys
import pandas as pd
import numpy as np
from sklearn.feature_extraction.text import TfidfVectorizer
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import Pipeline
from sklearn.model_selection import StratifiedKFold, cross_validate
import joblib

def train_tfidf(X, y):
    pipeline = Pipeline([
        ('tfidf', TfidfVectorizer(
            max_features=50000,
            ngram_range=(1, 3),
            sublinear_tf=True,
            min_df=3
        )),
        ('clf', LogisticRegression(
            C=1.0,
            class_weight='balanced',
            max_iter=1000,
            solver='lbfgs',
            n_jobs=-1
        ))
    ])

    print("Running 5-fold stratified cross-validation for TF-IDF baseline...")
    cv = StratifiedKFold(n_splits=5, shuffle=True, random_state=42)
    scoring = ['f1', 'precision', 'recall', 'roc_auc']
    
    cv_results = cross_validate(pipeline, X, y, cv=cv, scoring=scoring, n_jobs=-1)
    
    report = {
        "mean_f1": float(np.mean(cv_results['test_f1'])),
        "mean_precision": float(np.mean(cv_results['test_precision'])),
        "mean_recall": float(np.mean(cv_results['test_recall'])),
        "mean_roc_auc": float(np.mean(cv_results['test_roc_auc']))
    }

    print("\nCross-Validation Results (TF-IDF):")
    for metric, value in report.items():
        print(f"  {metric}: {value:.4f}")

    print("\nFitting final TF-IDF model on all data...")
    pipeline.fit(X, y)

    os.makedirs("models", exist_ok=True)
    model_path = "models/ml_classifier.joblib"
    report_path = "models/ml_classifier_report.json"
    
    joblib.dump(pipeline, model_path)
    print(f"Model saved to {model_path}")
    
    with open(report_path, 'w', encoding='utf-8') as f:
        json.dump(report, f, indent=2)
    print(f"Report saved to {report_path}")

def train_transformer(X, y, model_name="gerulata/slovakbert"):
    try:
        import torch
        from transformers import AutoTokenizer, AutoModelForSequenceClassification, Trainer, TrainingArguments
        from sklearn.model_selection import train_test_split
        from datasets import Dataset
    except ImportError as e:
        print(f"ERROR: Missing PyTorch or Transformers libraries. Cannot train transformer model. Details: {e}", file=sys.stderr)
        sys.exit(1)

    print(f"Preparing datasets for transformer fine-tuning ({model_name})...")
    
    # Stratified split for train / eval
    X_train, X_val, y_train, y_val = train_test_split(
        X, y, test_size=0.15, stratify=y, random_state=42
    )

    tokenizer = AutoTokenizer.from_pretrained(model_name)

    def tokenize_function(examples):
        return tokenizer(examples["text"], padding="max_length", truncation=True, max_length=256)

    train_df = pd.DataFrame({"text": X_train, "label": y_train})
    val_df = pd.DataFrame({"text": X_val, "label": y_val})

    train_dataset = Dataset.from_pandas(train_df)
    val_dataset = Dataset.from_pandas(val_df)

    train_dataset = train_dataset.map(tokenize_function, batched=True)
    val_dataset = val_dataset.map(tokenize_function, batched=True)

    print("Loading pre-trained transformer model...")
    model = AutoModelForSequenceClassification.from_pretrained(model_name, num_labels=2)

    training_args = TrainingArguments(
        output_dir="./models/transformer_checkpoints",
        evaluation_strategy="epoch",
        learning_rate=2e-5,
        per_device_train_batch_size=8,
        per_device_eval_batch_size=8,
        num_train_epochs=3,
        weight_decay=0.01,
        save_strategy="epoch",
        load_best_model_at_end=True,
        metric_for_best_model="f1",
        logging_steps=10,
        disable_tqdm=True
    )

    def compute_metrics(eval_pred):
        logits, labels = eval_pred
        predictions = np.argmax(logits, axis=-1)
        from sklearn.metrics import precision_recall_fscore_support, roc_auc_score
        precision, recall, f1, _ = precision_recall_fscore_support(labels, predictions, average='binary')
        # Apply softmax to get probability for class 1
        probs = np.exp(logits) / np.sum(np.exp(logits), axis=-1, keepdims=True)
        roc_auc = roc_auc_score(labels, probs[:, 1])
        return {
            "f1": f1,
            "precision": precision,
            "recall": recall,
            "roc_auc": roc_auc
        }

    trainer = Trainer(
        model=model,
        args=training_args,
        train_dataset=train_dataset,
        eval_dataset=val_dataset,
        compute_metrics=compute_metrics,
    )

    print("Starting training...")
    trainer.train()

    print("Evaluating model...")
    eval_results = trainer.evaluate()
    print("\nEvaluation Results (Transformer):")
    for metric, value in eval_results.items():
        print(f"  {metric}: {value:.4f}")

    # Save final model
    output_dir = "models/slovak_bert"
    os.makedirs(output_dir, exist_ok=True)
    model.save_pretrained(output_dir)
    tokenizer.save_pretrained(output_dir)
    print(f"Transformer model and tokenizer saved to {output_dir}")

    # Write report
    report = {
        "mean_f1": eval_results.get("eval_f1", 0.0),
        "mean_precision": eval_results.get("eval_precision", 0.0),
        "mean_recall": eval_results.get("eval_recall", 0.0),
        "mean_roc_auc": eval_results.get("eval_roc_auc", 0.0)
    }
    with open("models/slovak_bert_report.json", 'w', encoding='utf-8') as f:
        json.dump(report, f, indent=2)

def main():
    parser = argparse.ArgumentParser(description="Train LibreFang ML Classifier")
    parser.add_argument("--data", required=True, help="Path to CSV dataset (columns: text, label)")
    parser.add_argument("--model-type", choices=["tfidf", "transformer"], default="tfidf",
                        help="Model type: tfidf (Logistic Regression) or transformer (BERT)")
    parser.add_argument("--base-model", default="gerulata/slovakbert",
                        help="HuggingFace model name for transformer training (e.g. gerulata/slovakbert or deeppavlov/bert-base-bg-cs-pl-ru-cased)")
    args = parser.parse_args()

    print(f"Loading data from {args.data}...")
    df = pd.read_csv(args.data)
    
    if 'text' not in df.columns or 'label' not in df.columns:
        raise ValueError("Dataset must contain 'text' and 'label' columns.")

    X = df['text'].fillna("")
    y = df['label']

    if args.model_type == "transformer":
        train_transformer(X, y, model_name=args.base_model)
    else:
        train_tfidf(X, y)

if __name__ == "__main__":
    main()
