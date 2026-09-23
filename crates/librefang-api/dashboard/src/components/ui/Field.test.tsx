import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { Field } from "./Field";

describe("Field", () => {
  // #5246: a <label> forwards every click inside its bounds to its first
  // labelable control, so wrapping the children silently ate clicks meant for
  // composite widgets — picking an MCP server from the catalog never reached
  // the option's handler. The manifest form has a behavioural test for that;
  // this one pins the structure directly, because the primitive is now shared
  // with pages that have no such test.
  it("does not wrap its children in a label", () => {
    const { container } = render(
      <Field label="Workspace">
        <button type="button">pick me</button>
      </Field>,
    );

    const wrapper = container.firstElementChild;
    expect(wrapper?.tagName).toBe("DIV");
    // The button is not inside any <label>, so its click cannot be redirected.
    expect(wrapper?.querySelector("label")).toBeNull();
  });

  it("associates the label with the control when given htmlFor", () => {
    render(
      <Field label="Workspace" htmlFor="ws">
        <input id="ws" />
      </Field>,
    );
    // getByLabelText only resolves through a real label association — this is
    // the property the whole htmlFor prop exists to add back.
    expect(screen.getByLabelText("Workspace")).toBe(screen.getByRole("textbox"));
  });

  it("leaves the label unassociated when htmlFor is omitted", () => {
    render(
      <Field label="Workspace">
        <input />
      </Field>,
    );
    expect(screen.queryByLabelText("Workspace")).not.toBeInTheDocument();
  });

  it("marks a required label but leaves validation to the caller", () => {
    render(
      <Field label="Name" required>
        <input />
      </Field>,
    );
    expect(screen.getByText("*")).toBeInTheDocument();
    // `required` is decoration: the control is not marked, because the form
    // engine decides validity, not this component.
    expect(screen.getByRole("textbox")).not.toBeRequired();
  });

  it("renders the error only when the field is also marked invalid", () => {
    const { rerender } = render(
      <Field label="Name" error="Required">
        <input />
      </Field>,
    );
    // An error string on a field the form does not consider invalid is not
    // rendered — otherwise a stale message would outlive the state it described.
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    rerender(
      <Field label="Name" invalid error="Required" errorId="name-err">
        <input />
      </Field>,
    );
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("Required");
    expect(alert).toHaveAttribute("id", "name-err");
  });

  it("shows the hint next to the error rather than replacing it", () => {
    render(
      <Field label="Name" hint="Used in the URL" invalid error="Required">
        <input />
      </Field>,
    );
    expect(screen.getByText("Used in the URL")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("Required");
  });

  it("still renders a control when there is no label", () => {
    render(
      <Field label="">
        <input aria-label="bare" />
      </Field>,
    );
    // No label means no top margin on the control slot; the control stays.
    expect(screen.getByLabelText("bare")).toBeInTheDocument();
  });

  it("does not steal a click aimed at a button in its children", () => {
    let clicked = 0;
    render(
      <Field label="Tools">
        <button type="button" onClick={() => (clicked += 1)}>
          add
        </button>
      </Field>,
    );
    fireEvent.click(screen.getByRole("button", { name: "add" }));
    expect(clicked).toBe(1);
  });
});
