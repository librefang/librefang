import { expect, test } from "@playwright/test";

test("loads dashboard shell", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("LibreFang")).toBeVisible();
  await expect(page.getByRole("link", { name: "Overview" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Agents" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Sessions" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Approvals" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Comms" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Providers" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Channels" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Skills" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Hands" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Workflows" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Scheduler" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Goals" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Analytics" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Memory" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Runtime" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Logs" })).toBeVisible();

  await page.getByRole("link", { name: "Comms" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Comms" })).toBeVisible();

  await page.getByRole("link", { name: "Hands" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Hands" })).toBeVisible();

  await page.getByRole("link", { name: "Goals" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "Goals" })).toBeVisible();
});

test("shows the sign-in dialog when dashboard credentials are required", async ({ page }) => {
  await page.route("**/api/auth/dashboard-check", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ mode: "credentials" }),
    });
  });

  await page.route("**/api/version", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ version: "test", hostname: "devbox" }),
    });
  });

  await page.goto("/");

  await expect(page.getByText("Sign In Required")).toBeVisible();
  await expect(page.getByPlaceholder("Enter username...")).toBeVisible();
  await expect(page.getByPlaceholder("Enter password...")).toBeVisible();
});
