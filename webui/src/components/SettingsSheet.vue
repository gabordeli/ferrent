<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";

import * as api from "../api";
import { trapFocus } from "../focus";
import { t } from "../i18n";
import type { DaemonConfig } from "../types";

const props = defineProps<{
  canWrite: boolean;
}>();
const emit = defineEmits<{
  close: [];
  toast: [message: string];
}>();

const closeButton = ref<HTMLButtonElement | null>(null);
const dialogEl = ref<HTMLElement | null>(null);
let releaseFocus: (() => void) | null = null;

const config = ref<DaemonConfig | null>(null);
const draft = ref("");
const loading = ref(true);
const saving = ref(false);
const error = ref("");

const dirty = computed(
  () => config.value !== null && draft.value !== config.value.toml,
);
const locked = computed(() => config.value?.env_locked ?? []);

async function load(): Promise<void> {
  loading.value = true;
  error.value = "";
  try {
    config.value = await api.getDaemonConfig();
    draft.value = config.value.toml;
  } catch (err) {
    error.value = err instanceof Error ? err.message : t("toast.error");
  } finally {
    loading.value = false;
  }
}

/// Saves against the revision the editor started from; a stale revision is a
/// visible error, never a silent overwrite.
async function save(): Promise<void> {
  const current = config.value;
  if (!current || !props.canWrite) return;
  saving.value = true;
  error.value = "";
  try {
    await api.setDaemonConfig(draft.value, current.revision);
    emit("toast", t("settings.saved"));
    await load();
  } catch (err) {
    error.value = err instanceof Error ? err.message : t("toast.error");
  } finally {
    saving.value = false;
  }
}

function reload(): void {
  if (config.value) draft.value = config.value.toml;
}

onMounted(() => {
  closeButton.value?.focus();
  if (dialogEl.value) releaseFocus = trapFocus(dialogEl.value);
  void load();
});
onBeforeUnmount(() => releaseFocus?.());
</script>

<template>
  <Teleport to="body">
    <div
      class="sheet-backdrop"
      tabindex="-1"
      @click.self="emit('close')"
      @keydown.esc="emit('close')"
    >
      <dialog
        ref="dialogEl"
        open
        class="sheet settings-sheet"
        aria-modal="true"
        :aria-label="t('settings.title')"
        @keydown.esc="emit('close')"
      >
        <div class="sheet-handle" aria-hidden="true" />
        <header class="sheet-head">
          <div>
            <h2>{{ t("settings.title") }}</h2>
            <p v-if="config">
              {{ t("settings.revision") }} {{ config.revision }}
            </p>
          </div>
          <button
            ref="closeButton"
            class="icon-button"
            :aria-label="t('sheet.close')"
            @click="emit('close')"
          >
            ✕
          </button>
        </header>

        <p v-if="locked.length" class="settings-locked">
          {{ t("settings.envLocked") }}: {{ locked.join(", ") }}
        </p>

        <p v-if="loading" class="hint">{{ t("settings.loading") }}</p>

        <template v-else>
          <label class="settings-editor">
            <span>{{ t("settings.toml") }}</span>
            <textarea
              v-model="draft"
              spellcheck="false"
              rows="18"
              :readonly="!canWrite"
            />
          </label>

          <p v-if="error" class="detail-error" role="alert">{{ error }}</p>
          <p v-else-if="!canWrite" class="readonly-note">
            {{ t("auth.readonly") }}
          </p>

          <div v-if="canWrite" class="sheet-actions">
            <button
              class="action primary"
              :disabled="saving || !dirty"
              @click="save"
            >
              {{ saving ? t("settings.saving") : t("sheet.save") }}
            </button>
            <button class="action" :disabled="saving || !dirty" @click="reload">
              {{ t("settings.reload") }}
            </button>
          </div>
        </template>
      </dialog>
    </div>
  </Teleport>
</template>
