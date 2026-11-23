<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

import { trapFocus } from "../focus";
import { t } from "../i18n";

const emit = defineEmits<{ close: [] }>();

const closeButton = ref<HTMLButtonElement | null>(null);
const dialogEl = ref<HTMLElement | null>(null);
let releaseFocus: (() => void) | null = null;

const rows: { keys: string; label: string }[] = [
  { keys: "Space", label: "shortcuts.pauseResume" },
  { keys: "Delete", label: "shortcuts.remove" },
  { keys: "Ctrl+F", label: "shortcuts.search" },
  { keys: "Ctrl+R", label: "shortcuts.refresh" },
  { keys: "Ctrl+A", label: "shortcuts.selectAll" },
  { keys: "Ctrl+O", label: "shortcuts.add" },
  { keys: "↑ / ↓", label: "shortcuts.move" },
  { keys: "Home / End", label: "shortcuts.jump" },
  { keys: "Esc", label: "shortcuts.escape" },
  { keys: "?", label: "shortcuts.help" },
];

onMounted(() => {
  closeButton.value?.focus();
  if (dialogEl.value) releaseFocus = trapFocus(dialogEl.value);
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
        class="sheet shortcut-sheet"
        aria-modal="true"
        :aria-label="t('shortcuts.title')"
      >
        <div class="sheet-handle" aria-hidden="true" />
        <header class="sheet-head">
          <div>
            <h2>{{ t("shortcuts.title") }}</h2>
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
        <dl class="shortcut-list">
          <div v-for="row in rows" :key="row.keys">
            <dt><kbd>{{ row.keys }}</kbd></dt>
            <dd>{{ t(row.label) }}</dd>
          </div>
        </dl>
      </dialog>
    </div>
  </Teleport>
</template>
