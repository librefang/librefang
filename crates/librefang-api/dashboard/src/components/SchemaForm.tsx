import { useState, useCallback, useEffect, useMemo, useRef, memo } from "react";
import { useTranslation } from "react-i18next";
import { type RegistrySchema, type RegistrySchemaField, type RegistrySchemaSection } from "../api";
import { useRegistrySchema } from "../lib/queries/config";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { Button } from "./ui/Button";
import { Skeleton } from "./ui/Skeleton";
import { ErrorState } from "./ui/ErrorState";

interface SchemaFormProps {
  contentType: string;
  initialValues?: Record<string, unknown>;
  onSubmit: (values: Record<string, unknown>) => Promise<void>;
  onCancel: () => void;
  title: string;
  submitLabel?: string;
}

const textareaClass =
  "w-full rounded-lg border border-border-subtle bg-main px-3 py-2 text-xs font-mono outline-none focus:border-brand resize-y";

/**
 * Scope of the schema-driven form (#T2.5, decision D6):
 *
 * - Top-level fields (string, number, bool, array of scalars) get real controls.
 * - A nested object or a list of objects (`schema.sections`) is edited as JSON
 *   text in a textarea, validated on every keystroke.
 *
 * Nested sections are deliberately NOT rendered recursively: a generated form
 * that tries to draw every shape costs more UI than it saves, and a JSON box
 * keeps the whole schema reachable. If a screen ever needs nested controls, it
 * should be hand-built for the shapes that matter, not generated.
 */

function buildDefaults(
  schema: RegistrySchema,
  initialValues?: Record<string, unknown>,
): Record<string, unknown> {
  const values: Record<string, unknown> = {};

  for (const [key, field] of Object.entries(schema.fields ?? {})) {
    if (initialValues && key in initialValues) {
      values[key] = initialValues[key];
    } else if (field.default !== undefined) {
      values[key] = field.default;
    } else if (field.type === "bool") {
      values[key] = false;
    } else {
      values[key] = "";
    }
  }

  // Sections keep whatever the record already had; a section with no value
  // starts empty and is omitted from the payload until the operator types JSON.
  for (const sectionKey of Object.keys(schema.sections ?? {})) {
    if (initialValues && sectionKey in initialValues) {
      values[sectionKey] = initialValues[sectionKey];
    }
  }

  return values;
}

function hasInvalidNumberValue(value: unknown, required: boolean): boolean {
  if (value === undefined || value === null || value === "") {
    return required;
  }

  return typeof value !== "number" || Number.isNaN(value);
}

function validateRequired(
  schema: RegistrySchema,
  values: Record<string, unknown>,
): string[] {
  const errors: string[] = [];

  for (const [key, field] of Object.entries(schema.fields ?? {})) {
    const v = values[key];
    if (field.required && (v === undefined || v === null || v === "")) {
      errors.push(key);
    } else if (field.type === "number" && hasInvalidNumberValue(v, field.required ?? false)) {
      errors.push(key);
    }
  }

  return errors;
}

// Comma-separated array editor. The draft is local because a controlled input
// whose value is the parsed array drops the comma the moment it is typed.
const ArrayField = memo(function ArrayField({
  fieldKey,
  field,
  value,
  onChange,
  hasError,
}: {
  fieldKey: string;
  field: RegistrySchemaField;
  value: unknown;
  onChange: (value: unknown) => void;
  hasError: boolean;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(() =>
    Array.isArray(value) ? value.join(", ") : String(value ?? ""),
  );
  const lastEmitted = useRef<unknown>(value);

  useEffect(() => {
    if (value === lastEmitted.current) return;
    lastEmitted.current = value;
    setDraft(Array.isArray(value) ? value.join(", ") : String(value ?? ""));
  }, [value]);

  return (
    <div className="space-y-1">
      <Input
        label={field.required ? `${fieldKey} *` : fieldKey}
        value={draft}
        onChange={(e) => {
          const raw = e.target.value;
          setDraft(raw);
          const next = raw
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean);
          lastEmitted.current = next;
          onChange(next);
        }}
        placeholder={
          field.example != null ? String(field.example) : t("schema_form.comma_separated")
        }
        className={hasError ? "border-error" : ""}
      />
      {field.description && (
        <p className="text-[10px] text-text-dim/60 pl-0.5">{field.description}</p>
      )}
    </div>
  );
});

/** Required/number checks inside a nested object or list of objects. */
function validateSection(
  section: RegistrySchemaSection,
  value: unknown,
  pathPrefix: string,
  errors: string[],
): void {
  if (Array.isArray(value)) {
    value.forEach((item, idx) => validateSection(section, item, `${pathPrefix}[${idx}]`, errors));
    return;
  }
  if (!value || typeof value !== "object") return;
  const record = value as Record<string, unknown>;
  for (const [key, field] of Object.entries(section.fields ?? {})) {
    const v = record[key];
    if (field.required && (v === undefined || v === null || v === "")) {
      errors.push(`${pathPrefix}.${key}`);
    } else if (field.type === "number" && hasInvalidNumberValue(v, field.required ?? false)) {
      errors.push(`${pathPrefix}.${key}`);
    }
  }
  for (const [key, sub] of Object.entries(section.sections ?? {})) {
    if (key in record) validateSection(sub, record[key], `${pathPrefix}.${key}`, errors);
  }
}

function validateSections(
  schema: RegistrySchema,
  values: Record<string, unknown>,
): string[] {
  const errors: string[] = [];
  for (const [key, section] of Object.entries(schema.sections ?? {})) {
    const value = values[key];
    if (value !== undefined) validateSection(section, value, key, errors);
  }
  return errors;
}

// Single field renderer
const SchemaField = memo(function SchemaField({
  fieldKey,
  field,
  value,
  onChange,
  hasError,
}: {
  fieldKey: string;
  field: RegistrySchemaField;
  value: unknown;
  onChange: (value: unknown) => void;
  hasError: boolean;
}) {
  const { t } = useTranslation();

  // string with options -> Select
  if (field.type === "string" && field.options && field.options.length > 0) {
    return (
      <div className="space-y-1">
        <Select
          label={field.required ? `${fieldKey} *` : fieldKey}
          options={field.options.map((o) => ({ value: o, label: o }))}
          value={String(value ?? "")}
          onChange={(e) => onChange(e.target.value)}
          placeholder={t("schema_form.select_option")}
          className={hasError ? "border-error" : ""}
        />
        {field.description && (
          <p className="text-[10px] text-text-dim/60 pl-0.5">{field.description}</p>
        )}
      </div>
    );
  }

  // bool -> toggle checkbox
  if (field.type === "bool") {
    return (
      <div className="space-y-1">
        <label className="flex items-center gap-3 cursor-pointer group">
          <button
            type="button"
            role="checkbox"
            aria-checked={!!value}
            onClick={() => onChange(!value)}
            className={`
              relative w-10 h-5 rounded-full transition-colors duration-200 shrink-0
              ${value ? "bg-brand" : "bg-main border border-border-subtle"}
            `}
          >
            <span
              className={`
                absolute top-0.5 left-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform duration-200
                ${value ? "translate-x-5" : "translate-x-0"}
              `}
            />
          </button>
          <span className="text-xs font-bold text-text-main">
            {fieldKey}
            {field.required && <span className="text-error ml-0.5">*</span>}
          </span>
        </label>
        {field.description && (
          <p className="text-[10px] text-text-dim/60 pl-0.5">{field.description}</p>
        )}
      </div>
    );
  }

  // number
  if (field.type === "number") {
    return (
      <div className="space-y-1">
        <Input
          label={field.required ? `${fieldKey} *` : fieldKey}
          type="number"
          value={value === "" || value === undefined ? "" : String(value)}
          onChange={(e) => {
            const raw = e.target.value;
            if (raw === "") {
              onChange("");
            } else {
              const num = Number(raw);
              if (!Number.isNaN(num)) {
                onChange(num);
              }
            }
          }}
          placeholder={field.example != null ? String(field.example) : ""}
          className={hasError ? "border-error" : ""}
        />
        {field.description && (
          <p className="text-[10px] text-text-dim/60 pl-0.5">{field.description}</p>
        )}
      </div>
    );
  }

  // array -> comma-separated text, kept as text so separators survive typing
  if (field.type === "array") {
    return (
      <ArrayField
        fieldKey={fieldKey}
        field={field}
        value={value}
        onChange={onChange}
        hasError={hasError}
      />
    );
  }

  // default: string input
  return (
    <div className="space-y-1">
      <Input
        label={field.required ? `${fieldKey} *` : fieldKey}
        value={String(value ?? "")}
        onChange={(e) => onChange(e.target.value)}
        placeholder={field.example != null ? String(field.example) : ""}
        className={hasError ? "border-error" : ""}
      />
      {field.description && (
        <p className="text-[10px] text-text-dim/60 pl-0.5">{field.description}</p>
      )}
    </div>
  );
});

const pretty = (value: unknown): string =>
  value === undefined ? "" : JSON.stringify(value, null, 2);

/** A nested object or list of objects, edited as validated JSON. */
const SectionJsonEditor = memo(function SectionJsonEditor({
  sectionKey,
  section,
  value,
  invalid,
  resetKey,
  onChange,
  onValidityChange,
}: {
  sectionKey: string;
  section: RegistrySchemaSection;
  value: unknown;
  invalid: boolean;
  resetKey: string;
  onChange: (value: unknown) => void;
  onValidityChange: (sectionKey: string, message: string | null) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(() => pretty(value));
  const [error, setError] = useState<string | null>(null);
  const lastEmitted = useRef<unknown>(undefined);

  // Opening a different record resets the box even if both records have no
  // value for this section (reference comparison alone would miss undefined).
  useEffect(() => {
    lastEmitted.current = value;
    setDraft(pretty(value));
    setError(null);
    onValidityChange(sectionKey, null);
    // Only when the record changes; `value` is the fresh record's value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resetKey]);

  // Re-seed when the record itself changes (editing a different row), but not
  // when the change came from this textarea: reformatting under the cursor
  // would fight every keystroke.
  useEffect(() => {
    if (value === lastEmitted.current) return;
    lastEmitted.current = value;
    setDraft(pretty(value));
    if (value !== undefined) {
      setError(null);
      onValidityChange(sectionKey, null);
    }
  }, [value, sectionKey, onValidityChange]);

  const handleChange = (text: string) => {
    setDraft(text);
    if (text.trim() === "") {
      setError(null);
      onValidityChange(sectionKey, null);
      lastEmitted.current = undefined;
      onChange(undefined);
      return;
    }
    try {
      const parsed: unknown = JSON.parse(text);
      setError(null);
      onValidityChange(sectionKey, null);
      lastEmitted.current = parsed;
      onChange(parsed);
    } catch {
      const message = t("schema_form.json_invalid");
      setError(message);
      onValidityChange(sectionKey, message);
    }
  };

  const errorId = `schema-form-${sectionKey}-json-error`;

  return (
    <fieldset className="border border-border-subtle/50 rounded-xl overflow-hidden">
      <div className="px-4 py-3 bg-main/30">
        <label
          htmlFor={`schema-form-${sectionKey}-json`}
          className="text-xs font-black uppercase tracking-widest text-text-dim"
        >
          {sectionKey}
        </label>
        {section.description && (
          <p className="text-[10px] text-text-dim/60 mt-1">{section.description}</p>
        )}
      </div>
      <div className="p-4 space-y-1">
        <textarea
          id={`schema-form-${sectionKey}-json`}
          value={draft}
          onChange={(e) => handleChange(e.target.value)}
          rows={6}
          spellCheck={false}
          placeholder={section.repeatable ? "[]" : "{}"}
          aria-invalid={invalid || undefined}
          aria-describedby={error ? errorId : undefined}
          className={`${textareaClass} ${invalid ? "border-error" : ""}`}
        />
        {error && (
          <p id={errorId} className="text-[10px] text-error">
            {error}
          </p>
        )}
      </div>
    </fieldset>
  );
});

// Loading skeleton for the form
function FormSkeleton() {
  return (
    <div className="space-y-4 p-5">
      <Skeleton className="h-4 w-48" />
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-4 w-32" />
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-10 w-full" />
      <div className="flex gap-2 pt-4">
        <Skeleton className="h-10 flex-1" />
        <Skeleton className="h-10 w-24" />
      </div>
    </div>
  );
}

export function SchemaForm({
  contentType,
  initialValues,
  onSubmit,
  onCancel,
  title,
  submitLabel,
}: SchemaFormProps) {
  const { t } = useTranslation();
  const [submitting, setSubmitting] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);
  const [jsonErrors, setJsonErrors] = useState<Record<string, string>>({});
  const [submitError, setSubmitError] = useState<string | null>(null);
  const errorSet = useMemo(() => new Set(errors), [errors]);

  const schemaQuery = useRegistrySchema(contentType);

  const schema = schemaQuery.data;

  const defaultValues = useMemo(() => {
    if (!schema) return {};
    return buildDefaults(schema, initialValues);
  }, [schema, initialValues]);

  const [values, setValues] = useState<Record<string, unknown>>({});

  // Sync defaults into values when schema arrives or when the caller passes
  // structurally different initialValues (editing a different record).
  const initialValuesSnapshot = useMemo(
    () => (initialValues ? JSON.stringify(initialValues) : ""),
    [initialValues],
  );
  const lastSnapshot = useRef<string | null>(null);
  useEffect(() => {
    if (!schema) return;
    if (lastSnapshot.current === initialValuesSnapshot) return;
    lastSnapshot.current = initialValuesSnapshot;
    setValues(defaultValues);
    setErrors([]);
    setJsonErrors({});
    setSubmitError(null);
  }, [schema, defaultValues, initialValuesSnapshot]);

  const updateField = useCallback((key: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    setErrors((prev) => prev.filter((e) => e !== key));
  }, []);

  const updateSection = useCallback((sectionKey: string, newVal: unknown) => {
    setValues((prev) => {
      const next = { ...prev };
      if (newVal === undefined) delete next[sectionKey];
      else next[sectionKey] = newVal;
      return next;
    });
  }, []);

  const handleJsonValidity = useCallback((sectionKey: string, message: string | null) => {
    setJsonErrors((prev) => {
      const next = { ...prev };
      if (message) next[sectionKey] = message;
      else delete next[sectionKey];
      return next;
    });
  }, []);

  const handleSubmit = async () => {
    if (!schema) return;

    const validationErrors = [
      ...validateRequired(schema, values),
      ...validateSections(schema, values),
    ];
    if (validationErrors.length > 0) {
      setErrors(validationErrors);
      return;
    }
    if (Object.keys(jsonErrors).length > 0) {
      setErrors(Object.keys(jsonErrors));
      return;
    }

    setSubmitting(true);
    setSubmitError(null);
    try {
      await onSubmit(values);
    } catch (err) {
      setSubmitError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  };

  if (schemaQuery.isLoading) {
    return (
      <div>
        {title && (
          <div className="px-5 py-3 border-b border-border-subtle">
            <h3 className="text-sm font-bold">{title}</h3>
          </div>
        )}
        <FormSkeleton />
      </div>
    );
  }

  if (schemaQuery.isError) {
    return (
      <div>
        {title && (
          <div className="px-5 py-3 border-b border-border-subtle flex items-center justify-between">
            <h3 className="text-sm font-bold">{title}</h3>
          </div>
        )}
        <div className="p-5">
          <ErrorState
            message={
              schemaQuery.error instanceof Error
                ? schemaQuery.error.message
                : t("schema_form.load_error")
            }
            onRetry={() => schemaQuery.refetch()}
          />
          <div className="mt-4 flex justify-end">
            <Button variant="secondary" onClick={onCancel}>
              {t("common.cancel")}
            </Button>
          </div>
        </div>
      </div>
    );
  }

  if (!schema) return null;

  const topLevelFields = Object.entries(schema.fields ?? {});
  const sections = schema.sections ? Object.entries(schema.sections) : [];

  return (
    <div>
      {title && (
        <div className="px-5 py-3 border-b border-border-subtle">
          <h3 className="text-sm font-bold">{title}</h3>
          {schema.description && (
            <p className="text-[10px] text-text-dim/60 mt-0.5">{schema.description}</p>
          )}
        </div>
      )}

      <div className="p-5 space-y-4">
        {schema.description && !title && (
          <p className="text-[10px] text-text-dim/60">{schema.description}</p>
        )}

        {topLevelFields.map(([key, field]) => (
          <SchemaField
            key={key}
            fieldKey={key}
            field={field}
            value={values[key]}
            onChange={(v) => updateField(key, v)}
            hasError={errorSet.has(key)}
          />
        ))}

        {sections.map(([sKey, section]) => (
          <SectionJsonEditor
            key={sKey}
            sectionKey={sKey}
            section={section}
            value={values[sKey]}
            invalid={
              sKey in jsonErrors ||
              [...errorSet].some((e) => e === sKey || e.startsWith(`${sKey}.`) || e.startsWith(`${sKey}[`))
            }
            resetKey={initialValuesSnapshot}
            onChange={(v) => updateSection(sKey, v)}
            onValidityChange={handleJsonValidity}
          />
        ))}

        {errors.length > 0 && (
          <p className="text-xs text-error">
            {t("schema_form.required_missing", { count: errors.length })}
          </p>
        )}

        {submitError && <p className="text-xs text-error">{submitError}</p>}

        <div className="flex gap-2 pt-2">
          <Button
            variant="primary"
            className="flex-1"
            onClick={handleSubmit}
            isLoading={submitting}
            disabled={submitting || Object.keys(jsonErrors).length > 0}
          >
            {submitLabel ?? t("common.save")}
          </Button>
          <Button variant="secondary" onClick={onCancel} disabled={submitting}>
            {t("common.cancel")}
          </Button>
        </div>
      </div>
    </div>
  );
}
