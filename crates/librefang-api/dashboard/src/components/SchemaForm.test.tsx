import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RegistrySchema } from "../api";
import { SchemaForm } from "./SchemaForm";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key,
  }),
}));

const query = vi.hoisted(() => ({
  current: {} as Record<string, unknown>,
}));
vi.mock("../lib/queries/config", () => ({
  useRegistrySchema: () => query.current,
}));

const schema: RegistrySchema = {
  description: "Channel entry",
  fields: {
    name: { type: "string", required: true },
    tags: { type: "array" },
    enabled: { type: "bool" },
  },
  sections: {
    connection: {
      description: "Nested connection object",
      fields: {
        host: { type: "string" },
        port: { type: "number" },
      },
    },
  },
};

const renderForm = (initialValues?: Record<string, unknown>) => {
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  render(
    <SchemaForm
      contentType="channel"
      title="New entry"
      initialValues={initialValues}
      onSubmit={onSubmit}
      onCancel={() => {}}
    />,
  );
  return { onSubmit };
};

describe("SchemaForm — nested sections as JSON", () => {
  beforeEach(() => {
    query.current = {
      data: schema,
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    };
  });

  it("renders a nested object as pretty-printed JSON in a textarea", () => {
    renderForm({ connection: { host: "localhost", port: 8080 } });

    const editor = screen.getByRole("textbox", { name: "connection" });
    expect(editor.tagName).toBe("TEXTAREA");
    expect((editor as HTMLTextAreaElement).value).toBe(
      JSON.stringify({ host: "localhost", port: 8080 }, null, 2),
    );
  });

  it("blocks submit while the JSON is invalid and recovers when it parses", async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderForm({ name: "primary", connection: { host: "a" } });

    const editor = screen.getByRole("textbox", { name: "connection" });
    await user.clear(editor);
    await user.type(editor, "{{");

    expect(await screen.findByText("schema_form.json_invalid")).toBeInTheDocument();
    const save = screen.getByRole("button", { name: "common.save" });
    expect(save).toBeDisabled();
    await user.click(save);
    expect(onSubmit).not.toHaveBeenCalled();

    await user.clear(editor);
    await user.type(editor, '{{"host":"b"}');

    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({ name: "primary", connection: { host: "b" } }),
      ),
    );
  });

  it("keeps comma-separated arrays and omits an untouched section", async () => {
    const user = userEvent.setup();
    const { onSubmit } = renderForm({ name: "primary" });

    await user.type(screen.getByRole("textbox", { name: "tags" }), "a, b, c");
    await user.click(screen.getByRole("button", { name: "common.save" }));

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith({
        name: "primary",
        tags: ["a", "b", "c"],
        enabled: false,
      }),
    );
  });

  it("reseeds after an invalid draft when the next record has no value for the section", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    const form = (initialValues?: Record<string, unknown>) => (
      <SchemaForm
        contentType="channel"
        title="Edit entry"
        initialValues={initialValues}
        onSubmit={onSubmit}
        onCancel={() => {}}
      />
    );
    const { rerender } = render(form({ connection: { host: "first" } }));

    const editor = screen.getByRole("textbox", { name: "connection" });
    await user.clear(editor);
    await user.type(editor, "{{");
    expect(await screen.findByText("schema_form.json_invalid")).toBeInTheDocument();

    rerender(form({ name: "second" }));

    expect(
      (screen.getByRole("textbox", { name: "connection" }) as HTMLTextAreaElement).value,
    ).toBe("");
    expect(screen.queryByText("schema_form.json_invalid")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "common.save" })).toBeEnabled();
  });

  it("blocks submit when a required field inside the section is missing", async () => {
    query.current = {
      data: {
        fields: { name: { type: "string", required: true } },
        sections: { connection: { fields: { host: { type: "string", required: true } } } },
      },
      isLoading: false,
      isError: false,
      error: null,
      refetch: vi.fn(),
    };
    const user = userEvent.setup();
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <SchemaForm
        contentType="channel"
        title="New entry"
        initialValues={{ name: "entry", connection: {} }}
        onSubmit={onSubmit}
        onCancel={() => {}}
      />,
    );

    await user.click(screen.getByRole("button", { name: "common.save" }));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByText("schema_form.required_missing")).toBeInTheDocument();
  });

  it("reseeds the JSON box when the caller opens a different record", () => {
    const { rerender } = render(
      <SchemaForm
        contentType="channel"
        title="Edit entry"
        initialValues={{ connection: { host: "first" } }}
        onSubmit={vi.fn()}
        onCancel={() => {}}
      />,
    );
    expect(
      (screen.getByRole("textbox", { name: "connection" }) as HTMLTextAreaElement).value,
    ).toContain("first");

    rerender(
      <SchemaForm
        contentType="channel"
        title="Edit entry"
        initialValues={{ connection: { host: "second" } }}
        onSubmit={vi.fn()}
        onCancel={() => {}}
      />,
    );
    expect(
      (screen.getByRole("textbox", { name: "connection" }) as HTMLTextAreaElement).value,
    ).toContain("second");
  });
});
