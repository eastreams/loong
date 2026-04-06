import i18n from "i18next";
import LanguageDetector from "i18next-browser-languagedetector";
import { initReactI18next } from "react-i18next";
import enApp from "../assets/locales/en/app.json";
import enAbilities from "../assets/locales/en/abilities.json";
import enChat from "../assets/locales/en/chat.json";
import enDashboard from "../assets/locales/en/dashboard.json";
import zhApp from "../assets/locales/zh-CN/app.json";
import zhAbilities from "../assets/locales/zh-CN/abilities.json";
import zhChat from "../assets/locales/zh-CN/chat.json";
import zhDashboard from "../assets/locales/zh-CN/dashboard.json";

const resources = {
  en: {
    translation: {
      ...enApp,
      ...enAbilities,
      chat: enChat,
      dashboard: enDashboard,
    },
  },
  "zh-CN": {
    translation: {
      ...zhApp,
      ...zhAbilities,
      chat: zhChat,
      dashboard: zhDashboard,
    },
  },
};

i18n
  .use(LanguageDetector)
  .use(initReactI18next)
  .init({
    resources,
    fallbackLng: "en",
    interpolation: {
      escapeValue: false,
    },
    detection: {
      order: ["localStorage", "navigator"],
      caches: ["localStorage"],
    },
  });

i18n.on("languageChanged", (language) => {
  document.documentElement.lang = language;
});

if (document.documentElement) {
  document.documentElement.lang = i18n.language || "en";
}

export default i18n;
