export function buildEvenTrafficSplit(variantCount: number) {
  if (variantCount <= 0) return [];

  const baseShare = Math.floor(100 / variantCount);
  const remainder = 100 % variantCount;

  return Array.from(
    { length: variantCount },
    (_, index) => baseShare + (index < remainder ? 1 : 0),
  );
}
