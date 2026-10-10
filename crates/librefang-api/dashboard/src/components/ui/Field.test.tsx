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

  // Without `htmlFor` the label stays a `<span>`, and a `<span>` beside a
  // control names nothing — it is not part of the accessible-name computation
  // at all. What carries the name is the wrapper, as a `role="group"`: the
  // fieldset/legend pattern, and the same mechanism the finders get through
  // `ariaLabel`. Before that group existed for a labelled Field, 31 controls
  // across the agent form had no name from either source — the count is
  // measured in the live accessibility tree by the e2e guard, not here.
  it("names the wrapper's group when htmlFor is omitted", () => {
    render(
      <Field label="Workspace">
        <input />
      </Field>,
    );

    const group = screen.getByRole("group", { name: "Workspace" });
    expect(group.tagName).toBe("DIV");
    expect(group).toContainElement(screen.getByRole("textbox"));

    // The control is still not named *by the label* — the association is with
    // the group, which is what a screen reader announces when focus enters it.
    // `getByLabelText` resolves through `aria-labelledby`, so what it finds
    // here is the wrapper, not the input.
    expect(screen.getByLabelText("Workspace")).toBe(group);
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

  it("keeps the required asterisk out of the control's accessible name", () => {
    render(
      <Field label="Name" htmlFor="name" required>
        <input id="name" />
      </Field>,
    );
    expect(screen.getByText("*")).toHaveAttribute("aria-hidden", "true");
    // The accessible name is "Name", not "Name*". `getByRole` resolves it
    // through the accessibility tree (which drops `aria-hidden` decoration),
    // unlike `getByLabelText`, whose matcher reads raw text content.
    expect(screen.getByRole("textbox", { name: "Name" })).toBeInTheDocument();
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
