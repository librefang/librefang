const campaign = "utm_medium=partner&utm_campaign=librefang_everyapi";

export const EVERYAPI_PARTNER = {
  logoUrl: "/dashboard/everyapi-logo.png",
  pageUrl: `https://everyapi.ai/integrations/librefang?utm_source=librefang_dashboard&${campaign}`,
  websiteUrl: "https://everyapi.ai/",
} as const;
