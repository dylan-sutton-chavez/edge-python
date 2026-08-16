import { Footer, Layout, Navbar } from 'nextra-theme-docs'
import { Head } from 'nextra/components'
import { getPageMap } from 'nextra/page-map'
import { Inter, JetBrains_Mono } from 'next/font/google'
import { NavbarAutoCollapse } from '../components/NavbarAutoCollapse'
import 'nextra-theme-docs/style.css'
import '../globals.css'

// Inter for body text, JetBrains Mono for code.
const sansBody = Inter({
    subsets: ['latin'],
    weight: ['400', '500', '600', '700'],
    variable: '--font-sans-body',
    display: 'swap',
})
const monoCode = JetBrains_Mono({
    subsets: ['latin'],
    weight: ['400', '500', '600'],
    variable: '--font-mono-code',
    display: 'swap',
})

const DEFAULT_DESCRIPTION = 'Edge Python, a sandboxed Python subset that runs in the browser as WebAssembly and natively in the CLI, with full interpreter snapshots.'

// Per-page <title> comes from frontmatter via the catch-all's generateMetadata; this just supplies the suffix template and the fallback.
export const metadata = {
    title: { template: '%s – Edge Python', default: 'Edge Python' },
    description: DEFAULT_DESCRIPTION,
}

// Legacy Twitter icon in vector format. The explicit height matches the GitHub icon, without it the svg renders unsized.
const twitterIcon = (
    <svg viewBox="0 0 24 24" height="24" fill="currentColor" aria-label="Twitter">
        <path d="M23.953 4.57a10 10 0 01-2.825.775 4.958 4.958 0 002.163-2.723c-.951.555-2.005.959-3.127 1.184a4.92 4.92 0 00-8.384 4.482C7.69 8.095 4.067 6.13 1.64 3.162a4.822 4.822 0 00-.666 2.475c0 1.71.87 3.213 2.188 4.096a4.904 4.904 0 01-2.228-.616v.06a4.923 4.923 0 003.946 4.827 4.996 4.996 0 01-2.212.085 4.936 4.936 0 004.604 3.417 9.867 9.867 0 01-6.102 2.105c-.39 0-.779-.023-1.17-.067a13.995 13.995 0 007.557 2.209c9.053 0 13.998-7.496 13.998-13.985 0-.21 0-.42-.015-.63A9.935 9.935 0 0024 4.59z"/>
    </svg>
)

const navbar = (
    <Navbar logo={<span style={{ fontWeight: 600 }}>Edge Python</span>} projectLink="https://github.com/dylan-sutton-chavez/edge-python" chatLink="https://x.com/pythonedge" chatIcon={twitterIcon}/>
)

const footer = <Footer>Edge Python</Footer>

export default async function RootLayout({ children }) {
    return (
        <html lang="en" dir="ltr" className={`${sansBody.variable} ${monoCode.variable}`} suppressHydrationWarning>
            <Head color={{hue: { dark: 204, light: 212 }, saturation: 100, lightness: { dark: 55, light: 45 }}}>
                <link rel="icon" type="image/svg+xml" href="/static/favicon.svg" />
            </Head>
            <body>
                <Layout navbar={navbar} footer={footer} pageMap={await getPageMap()} docsRepositoryBase="https://github.com/dylan-sutton-chavez/edge-python/tree/main/docs">
                {children}
                </Layout>
                <NavbarAutoCollapse />
            </body>
        </html>
    )
}
