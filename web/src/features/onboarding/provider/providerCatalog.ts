export interface ProviderCatalogItem {
  kind: string;
  displayName: string;
  defaultBaseUrl: string;
  defaultChatPath: string;
  defaultModelsPath: string | null;
  defaultModel: string | null;
  recommendedOnboardingModel: string | null;
  authScheme: string;
  featureFamily: string;
  isCodingVariant: boolean;
  aliases: string[];
  configurationHint: string | null;
}

export function buildProviderKindOptions(
  catalog: ProviderCatalogItem[],
  selectedKind: string,
) {
  const options = catalog.map((item) => ({
    value: item.kind,
    label: item.displayName,
  }));

  if (selectedKind && !options.some((option) => option.value === selectedKind)) {
    options.unshift({ value: selectedKind, label: selectedKind });
  }

  return options;
}
