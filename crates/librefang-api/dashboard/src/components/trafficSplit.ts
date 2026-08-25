export const MAX_TRAFFIC_VARIANTS = 100;

export function buildEvenTrafficSplit(variantCount: number): number[] {
  if (variantCount <= 0) return [];
  if (variantCount > MAX_TRAFFIC_VARIANTS) {
    throw new Error(
      `variantCount (${variantCount}) cannot exceed ${MAX_TRAFFIC_VARIANTS}`,
    );
  }

  const baseShare = Math.floor(100 / variantCount);
  const remainder = 100 % variantCount;

  return Array.from(
    { length: variantCount },
    (_, index) => baseShare + (index < remainder ? 1 : 0),
  );
}
