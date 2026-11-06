package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.net.InetAddress
import java.net.UnknownHostException

/**
 * After the tablet rejoins Wi-Fi, `amd-pc.local` can stay unresolvable until Wi-Fi is
 * cycled (seen live: 0 of 15 lookups in one round, 15 of 15 in the next), and `amd-pc`
 * fails while the router answers instead of AdGuard.
 */
class ServerDnsTest {
    private val lan = InetAddress.getByName("192.168.0.249")

    private fun resolver(vararg answers: Pair<String, InetAddress>, store: AddressStore = MemoryAddressStore()): Pair<ServerDns, MutableList<String>> {
        val asked = mutableListOf<String>()
        val table = answers.toMap()
        return ServerDns(store) { name ->
            asked += name
            listOfNotNull(table[name]).ifEmpty { throw UnknownHostException(name) }
        } to asked
    }

    @Test
    fun aNameThatResolvesIsAskedOnce() {
        val (dns, asked) = resolver("amd-pc.local" to lan)
        assertEquals(listOf(lan), dns.lookup("amd-pc.local"))
        assertEquals(listOf("amd-pc.local"), asked)
        assertEquals("http://amd-pc.local:3333", dns.usableUrl("http://amd-pc.local:3333"))
    }

    @Test
    fun aLostMdnsNameFallsBackToTheUnicastOneAndBack() {
        assertEquals(lan, resolver("amd-pc" to lan).first.lookup("amd-pc.local").single())
        assertEquals(lan, resolver("amd-pc.local" to lan).first.lookup("amd-pc").single())
    }

    @Test
    fun aDottedNameIsNotGuessedAt() {
        assertEquals(listOf("example.com"), ServerDns.candidates("example.com"))
        assertEquals(listOf("odroid.house.local"), ServerDns.candidates("odroid.house.local"))
    }

    @Test
    fun whenNothingAnswersTheLastAddressThatDidIsUsed() {
        val store = MemoryAddressStore()
        resolver("amd-pc.local" to lan, store = store).first.lookup("amd-pc.local")
        val (dns, _) = resolver(store = store)
        assertEquals(listOf(lan), dns.lookup("amd-pc.local"))
        assertEquals("the page follows the app to the address", "http://192.168.0.249:3333/session/x",
            dns.usableUrl("http://amd-pc.local:3333/session/x"))
    }

    @Test
    fun aLinkLocalAddressIsNotRemembered() {
        val store = MemoryAddressStore()
        resolver("amd-pc.local" to InetAddress.getByName("fe80::1"), store = store).first.lookup("amd-pc.local")
        assertTrue(store.recall("amd-pc.local").isEmpty())
    }

    @Test(expected = UnknownHostException::class)
    fun aNameNeverSeenStillFailsLoudly() {
        resolver().first.lookup("amd-pc.local")
    }

    @Test
    fun anAddressIsDialledAsItIs() {
        val (dns, asked) = resolver("10.0.2.2" to InetAddress.getByName("10.0.2.2"))
        dns.lookup("10.0.2.2")
        assertEquals(listOf("10.0.2.2"), asked)
        assertEquals("http://10.0.2.2:4123", dns.usableUrl("http://10.0.2.2:4123"))
    }

    // A client built anywhere else resolves through the system alone and loses the server
    // exactly when the reader wakes the tablet.
    @Test
    fun everyHttpClientIsBuiltThroughHttpClients() {
        val sources = File("src/main/java/com/flakm/einkbridge").listFiles { f -> f.extension == "kt" }!!
        val strays = sources.filter { it.name != "HttpClients.kt" && "OkHttpClient.Builder()" in it.readText() }
        assertEquals(emptyList<String>(), strays.map { it.name })
    }
}
