import { useState, useCallback, useEffect, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { fetchRegistrySchema, type RegistrySchema, type RegistrySchemaField, type RegistrySchemaSection } from "../api";
import { Input } from "./ui/Input";
import { Select } from "./ui/Select";
import { Button } from "./ui/Button";
import { Skeleton } from "./ui/Skeleton";
import { ErrorState } from "./ui/ErrorState";
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react";

interface SchemaFormProps {
  contentType: string;
  initialValues?: Record<string, unknown>;
  onSubmit: (values: Record<string, unknown>) => Promise<void>;
  onCancel: () => void;
  title: string;
  submitLabel?: string;
}

// Recursively build defaults for a single section (including sub-sections)
function buildSectionDefaults(
  section: RegistrySchemaSection,
  initialValues?: Record<string, unknown>,
): Record<string, unknown> {
  const defaults: Record<string, unknown> = {};
  for (const [fKey, f] of Object.entries(section.fields ?? {})) {
    if (initialValues && fKey in initialValues) {
      defaults[fKey] = initialValues[fKey];
    } else if (f.default !== undefined) {
      defaults[fKey] = f.default;
    } else if (f.type === "bool") {
      defaults[fKey] = false;
    } else {
      defaults[fKey] = "";
    }
  }
  for (const [sKey, s] of Object.entries(section.sections ?? {})) {
    if (initialValues && sKey in initialValues) {
      defaults[sKey] = initialValues[sKey];
    } else if (s.repeatable) {
      defaults[sKey] = [];
    } else {
      defaults[sKey] = buildSectionDefaults(s);
    }
  }
  return defaults;
}

// Build initial form values from schema defaults and provided initialValues
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

  // Initialize sections — repeatable sections start with empty array,
  // non-repeatable sections get a single flat object with defaults (recursive)
  for (const [sectionKey, section] of Object.entries(schema.sections ?? {})) {
    if (initialValues && sectionKey in initialValues) {
      values[sectionKey] = initialValues[sectionKey];
    } else if (section.repeatable) {
      values[sectionKey] = [];
    } else {
      values[sectionKey] = buildSectionDefaults(
        section,
        initialValues?.[sectionKey] as Record<string, unknown> | undefined,
      );
    }
  }

  return values;
}

// Build a blank entry for a repeatable section (including sub-section defaults)
function blankSectionEntry(section: RegistrySchemaSection): Record<string, unknown> {
  const entry: Record<string, unknown> = {};
  for (const [key, field] of Object.entries(section.fields ?? {})) {
    if (field.default !== undefined) {
      entry[key] = field.default;
    } else if (field.type === "bool") {
      entry[key] = false;
    } else {
      entry[key] = "";
    }
  }
  for (const [sKey, s] of Object.entries(section.sections ?? {})) {
    if (s.repeatable) {
      entry[sKey] = [];
    } else {
      entry[sKey] = buildSectionDefaults(s);
    }
  }
  return entry;
}

// Recursively validate required fields within a section
function validateSectionRequired(
  section: RegistrySchemaSection,
  values: Record<string, unknown>,
  pathPrefix: string,
  errors: string[],
): void {
  for (const [fKey, f] of Object.entries(section.fields ?? {})) {
    const v = values[fKey];
    if (f.required && (v === undefined || v === null || v === "")) {
      errors.push(`${pathPrefix}.${fKey}`);
    } else if (f.type === "number" && typeof v === "string" && v !== "") {
      errors.push(`${pathPrefix}.${fKey}`);
    }
  }
  for (const [sKey, s] of Object.entries(section.sections ?? {})) {
    if (s.repeatable) {
      const items = values[sKey];
      if (Array.isArray(items)) {
        items.forEach((item, idx) => {
          validateSectionRequired(
            s,
            item as Record<string, unknown>,
            `${pathPrefix}.${sKey}[${idx}]`,
            errors,
          );
        });
      }
    } else {
      const subVal = values[sKey] as Record<string, unknown> | undefined;
      if (subVal) {
        validateSectionRequired(s, subVal, `${pathPrefix}.${sKey}`, errors);
      }
    }
  }
}

// Validate required fields; returns list of field paths that are missing
function validateRequired(
  schema: RegistrySchema,
  values: Record<string, unknown>,
): string[] {
  const errors: string[] = [];

  for (const [key, field] of Object.entries(schema.fields ?? {})) {
    const v = values[key];
    if (field.required && (v === undefined || v === null || v === "")) {
      errors.push(key);
    } else if (field.type === "number" && typeof v === "string" && v !== "") {
      errors.push(key);
    }
  }

  for (const [sectionKey, section] of Object.entries(schema.sections ?? {})) {
    if (section.repeatable) {
      const items = values[sectionKey];
      if (Array.isArray(items)) {
        items.forEach((item, idx) => {
          validateSectionRequired(
            section,
            item as Record<string, unknown>,
            `${sectionKey}[${idx}]`,
            errors,
          );
        });
      }
    } else {
      const sectionVal = values[sectionKey] as Record<string, unknown> | undefined;
      if (sectionVal) {
        validateSectionRequired(section, sectionVal, sectionKey, errors);
      }
    }
  }

  return errors;
}

// Single field renderer
function SchemaField({
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
          label={
            field.required
              ? `${fieldKey} *`
              : fieldKey
          }
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
            const num = Number(raw);
            onChange(raw === "" ? "" : Number.isNaN(num) ? raw : num);
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

  // array -> comma-separated text
  if (field.type === "array") {
    const arrValue = Array.isArray(value) ? value.join(", ") : String(value ?? "");
    return (
      <div className="space-y-1">
        <Input
          label={field.required ? `${fieldKey} *` : fieldKey}
          value={arrValue}
          onChange={(e) => {
            const raw = e.target.value;
            onChange(
              raw
                .split(",")
                .map((s) => s.trim())
                .filter(Boolean),
            );
          }}
          placeholder={field.example != null ? String(field.example) : t("schema_form.comma_separated")}
          className={hasError ? "border-error" : ""}
        />
        {field.description && (
          <p className="text-[10px] text-text-dim/60 pl-0.5">{field.description}</p>
        )}
      </div>
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
}

// Collapsible section fieldset (supports recursive sub-sections)
function SectionFieldset({
  sectionKey,
  section,
  values,
  errors,
  onChange,
  pathPrefix,
}: {
  sectionKey: string;
  section: RegistrySchemaSection;
  values: Record<string, unknown>;
  errors: string[];
  onChange: (sectionKey: string, newVal: unknown) => void;
  pathPrefix?: string;
}) {
  const { t } = useTranslation();
  const [collapsed, setCollapsed] = useState(false);
  const fullPath = pathPrefix ?? sectionKey;

  if (section.repeatable) {
    const items = (Array.isArray(values[sectionKey]) ? values[sectionKey] : []) as Record<string, unknown>[];

    const addItem = () => {
      onChange(sectionKey, [...items, blankSectionEntry(section)]);
    };

    const removeItem = (idx: number) => {
      onChange(sectionKey, items.filter((_, i) => i !== idx));
    };

    const updateItem = (idx: number, fieldKey: string, fieldValue: unknown) => {
      const updated = items.map((item, i) =>
        i === idx ? { ...item, [fieldKey]: fieldValue } : item,
      );
      onChange(sectionKey, updated);
    };

    return (
      <fieldset className="border border-border-subtle/50 rounded-xl overflow-hidden">
        <button
          type="button"
          className="w-full flex items-center gap-2 px-4 py-3 bg-main/30 hover:bg-main/50 transition-colors text-left"
          onClick={() => setCollapsed(!collapsed)}
        >
          {collapsed ? (
            <ChevronRight className="w-3.5 h-3.5 text-text-dim shrink-0" />
          ) : (
            <ChevronDown className="w-3.5 h-3.5 text-text-dim shrink-0" />
          )}
          <span className="text-xs font-black uppercase tracking-widest text-text-dim">
            {sectionKey}
          </span>
          <span className="text-[10px] text-text-dim/50 ml-1">
            ({items.length})
          </span>
        </button>

        {!collapsed && (
          <div className="p-4 space-y-3">
            {section.description && (
              <p className="text-[10px] text-text-dim/60">{section.description}</p>
            )}

            {items.map((item, idx) => (
              <div
                key={idx}
                className="p-3 rounded-lg border border-border-subtle/30 bg-surface space-y-3"
              >
                <div className="flex items-center justify-between">
                  <span className="text-[10px] font-bold text-text-dim uppercase tracking-widest">
                    #{idx + 1}
                  </span>
                  <button
                    type="button"
                    onClick={() => removeItem(idx)}
                    className="p-1 rounded hover:bg-error/10 text-text-dim hover:text-error transition-colors"
                    aria-label={t("common.delete")}
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
                {Object.entries(section.fields ?? {}).map(([fKey, f]) => (
                  <SchemaField
                    key={fKey}
                    fieldKey={fKey}
                    field={f}
                    value={item[fKey]}
                    onChange={(v) => updateItem(idx, fKey, v)}
                    hasError={errors.includes(`${fullPath}[${idx}].${fKey}`)}
                  />
                ))}
                {Object.entries(section.sections ?? {}).map(([subKey, subSection]) => (
                  <SectionFieldset
                    key={subKey}
                    sectionKey={subKey}
                    section={subSection}
                    values={item}
                    errors={errors}
                    onChange={(subSectionKey, newSubVal) => {
                      updateItem(idx, subSectionKey, newSubVal);
                    }}
                    pathPrefix={`${fullPath}[${idx}].${subKey}`}
                  />
                ))}
              </div>
            ))}

            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={addItem}
              className="w-full border border-dashed border-border-subtle"
            >
              <Plus className="w-3.5 h-3.5 mr-1" />
              {t("schema_form.add_item")}
            </Button>
          </div>
        )}
      </fieldset>
    );
  }

  // Non-repeatable section
  const sectionVal = (values[sectionKey] ?? {}) as Record<string, unknown>;

  const updateField = (fieldKey: string, fieldValue: unknown) => {
    onChange(sectionKey, { ...sectionVal, [fieldKey]: fieldValue });
  };

  return (
    <fieldset className="border border-border-subtle/50 rounded-xl overflow-hidden">
      <button
        type="button"
        className="w-full flex items-center gap-2 px-4 py-3 bg-main/30 hover:bg-main/50 transition-colors text-left"
        onClick={() => setCollapsed(!collapsed)}
      >
        {collapsed ? (
          <ChevronRight className="w-3.5 h-3.5 text-text-dim shrink-0" />
        ) : (
          <ChevronDown className="w-3.5 h-3.5 text-text-dim shrink-0" />
        )}
        <span className="text-xs font-black uppercase tracking-widest text-text-dim">
          {sectionKey}
        </span>
      </button>

      {!collapsed && (
        <div className="p-4 space-y-3">
          {section.description && (
            <p className="text-[10px] text-text-dim/60">{section.description}</p>
          )}
          {Object.entries(section.fields ?? {}).map(([fKey, f]) => (
            <SchemaField
              key={fKey}
              fieldKey={fKey}
              field={f}
              value={sectionVal[fKey]}
              onChange={(v) => updateField(fKey, v)}
              hasError={errors.includes(`${fullPath}.${fKey}`)}
            />
          ))}
          {Object.entries(section.sections ?? {}).map(([subKey, subSection]) => (
            <SectionFieldset
              key={subKey}
              sectionKey={subKey}
              section={subSection}
              values={sectionVal}
              errors={errors}
              onChange={(subSectionKey, newSubVal) => {
                onChange(sectionKey, { ...sectionVal, [subSectionKey]: newSubVal });
              }}
              pathPrefix={`${fullPath}.${subKey}`}
            />
          ))}
        </div>
      )}
    </fieldset>
  );
}

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
  const [submitError, setSubmitError] = useState<string | null>(null);

  const schemaQuery = useQuery({
    queryKey: ["registry-schema", contentType],
    queryFn: () => fetchRegistrySchema(contentType),
    staleTime: 5 * 60 * 1000,
    retry: 1,
  });

  const schema = schemaQuery.data;

  // Initialize values once schema loads
  const defaultValues = useMemo(() => {
    if (!schema) return {};
    return buildDefaults(schema, initialValues);
  }, [schema, initialValues]);

  const [values, setValues] = useState<Record<string, unknown>>({});

  // Sync defaults into values when schema first arrives
  const [initialized, setInitialized] = useState(false);
  useEffect(() => {
    if (schema && !initialized) {
      setValues(defaultValues);
      setInitialized(true);
    }
  }, [schema, initialized, defaultValues]);

  const updateField = useCallback((key: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    // Clear error for this field when user types
    setErrors((prev) => prev.filter((e) => e !== key));
  }, []);

  const updateSection = useCallback((sectionKey: string, newVal: unknown) => {
    setValues((prev) => ({ ...prev, [sectionKey]: newVal }));
    // Clear section errors (match "key." or "key[" to avoid prefix collisions)
    setErrors((prev) => prev.filter((e) =>
      e !== sectionKey && !e.startsWith(`${sectionKey}.`) && !e.startsWith(`${sectionKey}[`)
    ));
  }, []);

  const handleSubmit = async () => {
    if (!schema) return;

    const validationErrors = validateRequired(schema, values);
    if (validationErrors.length > 0) {
      setErrors(validationErrors);
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

  // Loading state
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

  // Error state
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

        {/* Top-level fields */}
        {topLevelFields.map(([key, field]) => (
          <SchemaField
            key={key}
            fieldKey={key}
            field={field}
            value={values[key]}
            onChange={(v) => updateField(key, v)}
            hasError={errors.includes(key)}
          />
        ))}

        {/* Sections */}
        {sections.map(([sKey, section]) => (
          <SectionFieldset
            key={sKey}
            sectionKey={sKey}
            section={section}
            values={values}
            errors={errors}
            onChange={updateSection}
          />
        ))}

        {/* Validation summary */}
        {errors.length > 0 && (
          <p className="text-xs text-error">
            {t("schema_form.required_missing", { count: errors.length })}
          </p>
        )}

        {/* Submit error */}
        {submitError && (
          <p className="text-xs text-error">{submitError}</p>
        )}

        {/* Actions */}
        <div className="flex gap-2 pt-2">
          <Button
            variant="primary"
            className="flex-1"
            onClick={handleSubmit}
            isLoading={submitting}
            disabled={submitting}
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
