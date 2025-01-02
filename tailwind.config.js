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
          "primary": "#FAF9F6",   //grey background for sensor boxes    
          "secondary": "#FFFFFF", //white
          "accent": "#1f1f1f",    //background grey    
          "neutral": "#1f1f1f",      
          "base-100": "#1f1f1f",         
          "info": "#FFFFFF",         
          "success": "#1f1f1f",         
          "warning": "#FFFFFF",         
          "error": "#a32634",
          
        }
      }
    ]
  },
  plugins: [require('daisyui')],
}

