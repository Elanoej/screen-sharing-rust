/**
 * Código `avc1.PPCCLL` a partir do AVCDecoderConfigurationRecord (avcC).
 * Os bytes 1..3 do record são profile, compatibilidade e level do SPS.
 */
export const codecFromAvcC = (description: Uint8Array): string | null => {
  if (description.length < 4) return null;
  const hex = (b: number) => b.toString(16).padStart(2, "0");
  return `avc1.${hex(description[1])}${hex(description[2])}${hex(description[3])}`;
};
