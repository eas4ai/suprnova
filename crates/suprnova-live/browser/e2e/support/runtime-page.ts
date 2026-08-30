import { expect, type Locator, type Page } from "@playwright/test";

export const ISLAND_SELECTOR = "[data-suprnova-live-island]";
export const STATUS_ATTRIBUTE = "data-suprnova-live-status";

export class RuntimePage {
  constructor(readonly page: Page) {}

  island(index = 0): Locator {
    return this.page.locator(ISLAND_SELECTOR).nth(index);
  }

  async open(scenario: string): Promise<void> {
    await this.page.goto(`/scenario/${scenario}`);
  }

  async expectVisibleContent(text: string): Promise<void> {
    await expect(this.page.getByText(text, { exact: true })).toBeVisible();
  }

  async expectStatus(status: "connected" | "invalid" | "incompatible", index = 0): Promise<void> {
    await expect(this.island(index)).toHaveAttribute(STATUS_ATTRIBUTE, status);
  }
}
