import { describe, expect, it } from "vitest";

import {
  APP_BUNDLE_IDENTIFIER,
  APP_DATABASE_FILENAME,
  APP_LOGO_ALT,
  APP_NAME,
  APP_RUNTIME_DIR,
  EXPORT_PREFIX,
  ONBOARDING_DRAFT_KEY,
} from "./appIdentity";

describe("appIdentity", () => {
  it("exports the CapyInn app identity constants", () => {
    expect(APP_NAME).toBe("CapyInn");
    expect(APP_LOGO_ALT).toBe("CapyInn logo");
    expect(EXPORT_PREFIX).toBe("CapyInn");
    expect(ONBOARDING_DRAFT_KEY).toBe("capyinn-onboarding-draft");
    expect(APP_RUNTIME_DIR).toBe("CapyInn");
    expect(APP_DATABASE_FILENAME).toBe("capyinn.db");
    expect(APP_BUNDLE_IDENTIFIER).toBe("io.capyinn.app");
  });
});
