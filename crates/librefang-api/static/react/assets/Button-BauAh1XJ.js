import{r as h,j as s}from"./router-CqrjJLBC.js";import{L as m}from"./loader-circle-uqxfEv0z.js";const l={primary:"bg-brand text-white hover:brightness-110 shadow-md shadow-brand/20 hover:shadow-lg hover:shadow-brand/30",secondary:"border border-border-subtle bg-surface text-text-main hover:bg-main/50 hover:border-brand/20 shadow-sm",ghost:"bg-transparent text-text-dim hover:text-text-main hover:bg-main/30",danger:"bg-error text-white hover:brightness-110 shadow-md shadow-error/20",success:"bg-success text-white hover:brightness-110 shadow-md shadow-success/20"},x={sm:"px-3 py-1.5 text-xs",md:"px-4 py-2 text-sm",lg:"px-6 py-3 text-base"},u=h.forwardRef(({className:t="",variant:r="primary",size:a="md",leftIcon:o,rightIcon:n,isLoading:e,disabled:d,children:i,...c},b)=>s.jsxs("button",{ref:b,disabled:d||e,className:`
          inline-flex items-center justify-center gap-2 rounded-xl font-bold
          transition-all duration-[400ms] ease-[cubic-bezier(0.22,1,0.36,1)]
          active:scale-[0.96] active:duration-100
          focus:outline-none focus:ring-2 focus:ring-brand/30 focus:ring-offset-1
          disabled:opacity-50 disabled:cursor-not-allowed disabled:active:scale-100
          ${l[r]}
          ${x[a]}
          ${t}
        `,...c,children:[e?s.jsx(m,{className:"h-4 w-4 animate-spin"}):o,i,n]}));u.displayName="Button";export{u as B};
