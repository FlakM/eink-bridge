package com.flakm.einkbridge

import android.content.Context
import okhttp3.Dns
import java.net.InetAddress
import java.net.URI
import java.net.UnknownHostException

/** Where the last addresses that answered for a name are kept. */
internal interface AddressStore {
    fun recall(host: String): List<String>
    fun remember(host: String, addresses: List<String>)
}

internal class MemoryAddressStore : AddressStore {
    private val known = mutableMapOf<String, List<String>>()
    override fun recall(host: String) = synchronized(known) { known[host].orEmpty() }
    override fun remember(host: String, addresses: List<String>) = synchronized(known) { known[host] = addresses }
}

internal class PrefsAddressStore(context: Context) : AddressStore {
    private val prefs = context.getSharedPreferences("server_addresses", Context.MODE_PRIVATE)
    override fun recall(host: String) = prefs.getString(host, null)?.split(',')?.filter { it.isNotBlank() }.orEmpty()
    override fun remember(host: String, addresses: List<String>) {
        prefs.edit().putString(host, addresses.joinToString(",")).apply()
    }
}

/**
 * Finds the server when the tablet's resolver will not.
 *
 * After the tablet wakes or rejoins Wi-Fi, `amd-pc.local` (mDNS) can stay unresolvable
 * until Wi-Fi is cycled, and `amd-pc` fails while the resolver asks the router, which
 * does not know it. So a single-label name is also tried as `.local` (and the reverse),
 * and when neither answers, the last addresses that did are used.
 */
internal class ServerDns(
    private val store: AddressStore,
    private val system: (String) -> List<InetAddress> = { Dns.SYSTEM.lookup(it) },
) : Dns {
    /** Names the system resolver failed on at their last lookup. */
    private val lost = java.util.concurrent.ConcurrentHashMap.newKeySet<String>()

    override fun lookup(hostname: String): List<InetAddress> {
        if (isLiteral(hostname)) return system(hostname)
        for (name in candidates(hostname)) {
            val found = runCatching { system(name) }.getOrNull().orEmpty()
            if (found.isNotEmpty()) {
                if (name == hostname) lost.remove(hostname) else lost.add(hostname)
                remember(hostname, found)
                return found
            }
        }
        lost.add(hostname)
        val known = store.recall(hostname).mapNotNull { runCatching { InetAddress.getByName(it) }.getOrNull() }
        if (known.isNotEmpty()) return known
        throw UnknownHostException("$hostname: no answer from DNS or mDNS, and no address that worked before")
    }

    /**
     * The URL with its host replaced by the address that last answered, when the system
     * resolver lost the name at its last lookup. For the WebView, which resolves names
     * itself; it never blocks, the app's own requests keep the state current.
     */
    fun usableUrl(url: String): String {
        val host = runCatching { URI(url).host }.getOrNull() ?: return url
        if (host !in lost) return url
        val literal = store.recall(host).firstOrNull() ?: return url
        val shown = if (':' in literal) "[$literal]" else literal
        return url.replaceFirst("://$host", "://$shown")
    }

    private fun remember(hostname: String, found: List<InetAddress>) {
        // A link-local address needs its interface to be dialled again; it is not worth keeping.
        val keep = found.filterNot { it.isLinkLocalAddress || it.isLoopbackAddress }.mapNotNull { it.hostAddress }
        if (keep.isNotEmpty()) store.remember(hostname, keep)
    }

    companion object {
        @Volatile var current: ServerDns = ServerDns(MemoryAddressStore())

        /** What every client in the app resolves through; follows [current]. */
        val shared: Dns = object : Dns {
            override fun lookup(hostname: String) = current.lookup(hostname)
        }

        fun install(context: Context) {
            current = ServerDns(PrefsAddressStore(context.applicationContext))
        }

        internal fun candidates(host: String): List<String> = when {
            host.endsWith(".local") && host.count { it == '.' } == 1 -> listOf(host, host.removeSuffix(".local"))
            '.' !in host -> listOf(host, "$host.local")
            else -> listOf(host)
        }

        private fun isLiteral(host: String) =
            host.firstOrNull()?.let { it.isDigit() || it == '[' || ':' in host } == true &&
                host.all { it.isDigit() || it == '.' || it == ':' || it in 'a'..'f' || it in 'A'..'F' || it == '[' || it == ']' }
    }
}
