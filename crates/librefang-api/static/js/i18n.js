(function(global) {
  'use strict';

  var SUPPORTED_LANGUAGES = ['en', 'zh-CN', 'ja'];
  var DEFAULT_LANGUAGE = 'en';
  var STORAGE_KEY = 'librefang-language';

  var currentLanguage = DEFAULT_LANGUAGE;
  var translations = {};
  var loaded = false;
  var observer = null;

  function detectLanguage() {
    var stored = localStorage.getItem(STORAGE_KEY);
    if (stored && SUPPORTED_LANGUAGES.indexOf(stored) >= 0) return stored;

    var browserLang = navigator.language || navigator.userLanguage || DEFAULT_LANGUAGE;
    if (SUPPORTED_LANGUAGES.indexOf(browserLang) >= 0) return browserLang;

    var langPrefix = browserLang.split('-')[0];
    for (var i = 0; i < SUPPORTED_LANGUAGES.length; i++) {
      if (SUPPORTED_LANGUAGES[i].split('-')[0] === langPrefix) return SUPPORTED_LANGUAGES[i];
    }
    return DEFAULT_LANGUAGE;
  }

  async function loadTranslations(lang) {
    try {
      var response = await fetch('/locales/' + lang + '.json');
      if (!response.ok) throw new Error('HTTP ' + response.status);
      return await response.json();
    } catch (error) {
      console.warn('[LibreFang i18n] failed to load locale', lang, error);
      if (lang !== DEFAULT_LANGUAGE) return loadTranslations(DEFAULT_LANGUAGE);
      return {};
    }
  }

  function getNestedValue(obj, path) {
    return path.split('.').reduce(function(current, key) {
      return current && current[key] !== undefined ? current[key] : null;
    }, obj);
  }

  function replacePlaceholders(str, params) {
    if (!params || typeof str !== 'string') return str;
    return str.replace(/\{(\w+)\}/g, function(match, key) {
      return params[key] !== undefined ? params[key] : match;
    });
  }

  function t(key, params) {
    var value = getNestedValue(translations, key);
    if (value === null) return '[' + key + ']';
    return replacePlaceholders(value, params);
  }

  function tOr(key, fallback, params) {
    var translated = t(key, params);
    if (!translated || translated.charAt(0) === '[') {
      return replacePlaceholders(fallback || key, params);
    }
    return translated;
  }

  function bindPageLanguage(page) {
    if (!page) return function() {};
    if (typeof page._unbindPageLanguage === 'function') return page._unbindPageLanguage;

    var disposed = false;

    function syncLanguage(event) {
      if (event && event.detail && event.detail.language) {
        page._currentLang = event.detail.language;
        return;
      }
      page._currentLang = currentLanguage;
    }

    syncLanguage();
    window.addEventListener('i18n-loaded', syncLanguage);
    window.addEventListener('i18n-changed', syncLanguage);

    function disposeLanguageBinding() {
      if (disposed) return;
      disposed = true;
      window.removeEventListener('i18n-loaded', syncLanguage);
      window.removeEventListener('i18n-changed', syncLanguage);
      if (page._unbindPageLanguage === disposeLanguageBinding) {
        delete page._unbindPageLanguage;
      }
    }

    page._unbindPageLanguage = disposeLanguageBinding;

    if (!page._pageLanguageDestroyWrapped) {
      var originalDestroy = page.destroy;
      page.destroy = function() {
        if (typeof this._unbindPageLanguage === 'function') {
          this._unbindPageLanguage();
        }
        if (typeof originalDestroy === 'function') {
          return originalDestroy.apply(this, arguments);
        }
      };
      page._pageLanguageDestroyWrapped = true;
    }

    return disposeLanguageBinding;
  }

  function tReactive(page, key, fallback, params) {
    // Touch the current language so Alpine re-runs bindings after locale changes.
    if (page) page._currentLang;
    return tOr(key, fallback, params);
  }

  function updateElement(element) {
    if (!element || element.nodeType !== 1) return;

    if (element.hasAttribute('data-i18n')) {
      var translated = t(element.getAttribute('data-i18n'));
      if (!translated.startsWith('[')) element.textContent = translated;
    }

    if (element.hasAttribute('data-i18n-placeholder')) {
      var translated = t(element.getAttribute('data-i18n-placeholder'));
      if (!translated.startsWith('[')) element.placeholder = translated;
    }

    if (element.hasAttribute('data-i18n-title')) {
      var translated = t(element.getAttribute('data-i18n-title'));
      if (!translated.startsWith('[')) element.title = translated;
    }
  }

  function updateTree(root) {
    if (!root) return;
    if (root.nodeType === 1) updateElement(root);
    if (typeof root.querySelectorAll !== 'function') return;

    root.querySelectorAll('[data-i18n]').forEach(updateElement);
    root.querySelectorAll('[data-i18n-placeholder]').forEach(updateElement);
    root.querySelectorAll('[data-i18n-title]').forEach(updateElement);
  }

  function updateDOM() {
    updateTree(document.body || document.documentElement || document);
  }

  function updateDocumentLanguage(lang) {
    if (document && document.documentElement) {
      document.documentElement.lang = lang || DEFAULT_LANGUAGE;
    }
  }

  function ensureObserver() {
    if (observer || typeof MutationObserver === 'undefined') return;
    var target = document.body || document.documentElement;
    if (!target) return;

    observer = new MutationObserver(function(mutations) {
      mutations.forEach(function(mutation) {
        mutation.addedNodes.forEach(function(node) {
          updateTree(node);
        });
      });
    });

    observer.observe(target, { childList: true, subtree: true });
  }

  async function init(lang) {
    currentLanguage = lang || detectLanguage();
    updateDocumentLanguage(currentLanguage);
    translations = await loadTranslations(currentLanguage);
    localStorage.setItem(STORAGE_KEY, currentLanguage);
    loaded = true;
    updateDOM();
    ensureObserver();
    window.dispatchEvent(new CustomEvent('i18n-loaded', { detail: { language: currentLanguage } }));
    return currentLanguage;
  }

  async function setLanguage(lang) {
    if (SUPPORTED_LANGUAGES.indexOf(lang) < 0) return false;
    currentLanguage = lang;
    updateDocumentLanguage(currentLanguage);
    translations = await loadTranslations(lang);
    localStorage.setItem(STORAGE_KEY, currentLanguage);
    updateDOM();
    window.dispatchEvent(new CustomEvent('i18n-changed', { detail: { language: currentLanguage } }));
    return true;
  }

  global.i18n = {
    init: init,
    t: t,
    bindPageLanguage: bindPageLanguage,
    tReactive: tReactive,
    setLanguage: setLanguage,
    getLanguage: function() { return currentLanguage; },
    getSupportedLanguages: function() { return SUPPORTED_LANGUAGES.slice(); },
    updateDOM: updateDOM,
    isReady: function() { return loaded; },
    DEFAULT_LANGUAGE: DEFAULT_LANGUAGE,
    SUPPORTED_LANGUAGES: SUPPORTED_LANGUAGES.slice()
  };
})(typeof window !== 'undefined' ? window : this);
