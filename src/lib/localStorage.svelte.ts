import { browser } from '$app/environment';

type Storage<T> = { value: T };

export function useLocalStorage<T>(key: string, initialValue: T): Storage<T> {
  let storage = $state({ value: initialValue });

  if (browser) {
    const item = localStorage.getItem(key);
    if (item) storage.value = JSON.parse(item) as T;
  }
  
  $effect(() => {
    
    localStorage.setItem(key, JSON.stringify(storage.value));
  });

  return storage;
}

export function getLocalImage(key: string, value: string) {
  let storage = $state({value});

  if(browser) {
    const item = localStorage.getItem(key);
    if(item) storage.value = item;
  }

  $effect (() => {
    localStorage.setItem(key, storage.value);
  })

  return storage;
}