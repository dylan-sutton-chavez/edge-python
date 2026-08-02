import { Footer, Layout, Navbar } from 'nextra-theme-docs'
import { Head } from 'nextra/components'
import { getPageMap } from 'nextra/page-map'
import { Inter, JetBrains_Mono } from 'next/font/google'
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

const navbar = (
    <Navbar logo={<span style={{ fontWeight: 600 }}>Edge Python</span>} projectLink="https://github.com/dylan-sutton-chavez/edge-python"/>
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
            </body>
        </html>
    )
}
