import { queryOptions, useQuery } from "@tanstack/react-query";
import {
  getFullConfig,
  getConfigSchema,
  fetchRegistrySchema,
  getRawConfigToml,
} from "../http/client";
import {
  type ConfigFieldSchema,
  type ConfigSchemaRoot,
  type ConfigSectionSchema,
  type JsonSchema,
  type UiFieldOptions,
  resolveRef,
} from "../../api";
import { configKeys, registryKeys } from "./keys";
import { withOverrides, type QueryOverrides } from "./options";

const STALE_MS = 60_000;
const SCHEMA_STALE_MS = 300_000;
const RAW_STALE_MS = 5_000;

/**
 * Build the legacy `{sections: {...}}` view model the ConfigPage renders
 * from, using only draft-07 JSON Schema input. This is a *view-model
 * projection*, not a wire-format adapter: the API now serves the raw
 * draft-07 schema and this mapping lives entirely client-side.
 *
 * When ConfigPage.tsx is rewritten to walk draft-07 directly, delete this.
 */
function projectDraft07Schema(
  root: ConfigSchemaRoot,
): { sections: Record<string, ConfigSectionSchema> } {
  const sections: Record<string, ConfigSectionSchema> = {};
  const descriptors = root["x-sections"] ?? [];
  const uiOptions = root["x-ui-options"] ?? {};

  const mapProperty = (
    prop: JsonSchema,
    ui?: UiFieldOptions,
  ): string | ConfigFieldSchema => {
    if (ui?.select) return { type: "select", options: ui.select };
    if (ui?.select_objects)
      return { type: "select", options: ui.select_objects };
    if (ui?.number_select)
      return { type: "number_select", options: ui.number_select };

    const primary = Array.isArray(prop.type)
      ? prop.type.find((t) => t !== "null") ?? prop.type[0]
      : prop.type;

    if (Array.isArray(prop.enum) && prop.enum.length > 0) {
      return { type: "select", options: prop.enum as string[] };
    }

    if (primary === "boolean") return "boolean";
    if (primary === "array") {
      return prop.items?.type === "string" ? "string[]" : "array";
    }
    if (primary === "object") return "object";
    if (primary === "integer" || primary === "number") {
      const f: ConfigFieldSchema = { type: "number" };
      if (ui?.min !== undefined) f.min = ui.min;
      else if (prop.minimum !== undefined) f.min = prop.minimum;
      if (ui?.max !== undefined) f.max = ui.max;
      else if (prop.maximum !== undefined) f.max = prop.maximum;
      if (ui?.step !== undefined) f.step = ui.step;
      else if (prop.multipleOf !== undefined) f.step = prop.multipleOf;
      return f;
    }
    return "string";
  };

  for (const desc of descriptors) {
    const fields: Record<string, string | ConfigFieldSchema> = {};

    if (desc.root_level && desc.fields) {
      for (const name of desc.fields) {
        const prop = root.properties?.[name];
        if (!prop) continue;
        fields[name] = mapProperty(prop, uiOptions[`/${name}`]);
      }
    } else if (desc.struct_field) {
      let target: JsonSchema | undefined = root.properties?.[desc.struct_field];
      if (target?.$ref) target = resolveRef(root, target.$ref);
      if (target?.properties) {
        for (const [name, prop] of Object.entries(target.properties)) {
          const ui = uiOptions[`/${desc.struct_field}/${name}`];
          fields[name] = mapProperty(prop, ui);
        }
      }
    }

    sections[desc.key] = {
      fields,
      root_level: desc.root_level,
      hot_reloadable: desc.hot_reloadable,
    };
  }

  return { sections };
}

export const configQueries = {
  full: () =>
    queryOptions({
      queryKey: configKeys.full(),
      queryFn: getFullConfig,
      staleTime: STALE_MS,
    }),
  schema: () =>
    queryOptions({
      queryKey: configKeys.schema(),
      queryFn: () => getConfigSchema().then(projectDraft07Schema),
      staleTime: SCHEMA_STALE_MS,
    }),
  registrySchema: (contentType: string) =>
    queryOptions({
      queryKey: registryKeys.schema(contentType),
      queryFn: () => fetchRegistrySchema(contentType),
      enabled: !!contentType,
      staleTime: SCHEMA_STALE_MS,
      retry: 1,
    }),
  rawToml: (enabled: boolean) =>
    queryOptions({
      queryKey: configKeys.rawToml(),
      queryFn: getRawConfigToml,
      enabled,
      staleTime: RAW_STALE_MS,
    }),
};



export function useFullConfig(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.full(), options));
}

export function useConfigSchema(options: QueryOverrides = {}) {
  return useQuery(withOverrides(configQueries.schema(), options));
}

export function useRegistrySchema(contentType: string, options: QueryOverrides = {}) {
  // Empty contentType disables query (enabled gate in configQueries)
  return useQuery(withOverrides(configQueries.registrySchema(contentType), options));
}

// Raw config.toml as text. Disabled by default — caller passes
// `enabled: true` only when the viewer modal is open. Short staleTime
// so re-opening shortly after a save reflects the change.
export function useRawConfigToml(enabled: boolean) {
  return useQuery(configQueries.rawToml(enabled));
}
