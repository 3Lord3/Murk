<script setup lang="ts">
// A single MDI icon: path from @mdi/js, size and color come from the outer class.
defineProps<{ path: string }>();
</script>

<template>
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path :d="path" />
  </svg>
</template>
