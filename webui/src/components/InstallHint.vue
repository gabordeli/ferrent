<script setup lang="ts">
import { ref } from "vue";

import { t } from "../i18n";
import { dismissInstall, shouldOfferInstall } from "../install";

const visible = ref(shouldOfferInstall());

function close(): void {
  visible.value = false;
  dismissInstall();
}
</script>

<template>
  <output v-if="visible" class="install-hint" role="status">
    <span>{{ t("install.hint") }}</span>
    <button
      class="icon-button compact"
      :aria-label="t('install.dismiss')"
      :title="t('install.dismiss')"
      @click="close"
    >
      ✕
    </button>
  </output>
</template>
