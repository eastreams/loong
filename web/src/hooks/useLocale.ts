import { useMemo } from "react";
import { useTranslation } from "react-i18next";

export function useLocale() {
  const { i18n } = useTranslation();

  return useMemo(
    () => ({
      locale: i18n.language,
      toggleLocale: () => {
        void i18n.changeLanguage(i18n.language === "en" ? "zh-CN" : "en");
      },
    }),
    [i18n],
  );
}
