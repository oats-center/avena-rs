/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{html,js,svelte,ts}'],
  theme: {
    extend: {},
  },
  daisyui: {
    themes: [
      {
        mytheme: {
          "primary": "#FFFFFF",      
          "secondary": "#000000",
          "accent": "#FFFFFF",         
          "neutral": "#FFFFFF",         
          "base-100": "#1f1f1f",         
          "info": "#FFFFFF",         
          "success": "#218234",         
          "warning": "#FFFFFF",         
          "error": "#a32634",
          
        }
      }
    ]
  },
  plugins: [require('daisyui')],
}

