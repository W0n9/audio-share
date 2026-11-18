package dev.audioshare.v2

import org.junit.Test
import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Resource-key parity seam: the default and zh-CN files must define the same
 * `<string>` and `<plurals>` names with the same format placeholders. Unit
 * tests run with the module as the working directory, so `src/main/res` is
 * resolved directly (with an `app/` fallback for IDE runners).
 */
class StringResourceParityTest {

    private fun resFile(relative: String): File {
        val candidates = listOf(
            File("src/main/res/$relative"),
            File("app/src/main/res/$relative"),
        )
        return candidates.firstOrNull(File::isFile)
            ?: error("resource file not found: $relative (cwd=${File(".").absolutePath})")
    }

    private data class ResourceKeys(
        val placeholders: Map<String, Set<String>>,
    ) {
        val names: Set<String> get() = placeholders.keys
    }

    private val placeholderPattern = Regex("""%(\d+\$)?[a-zA-Z]""")

    private fun placeholders(text: String): Set<String> =
        placeholderPattern.findAll(text).map { it.value }.toSet()

    private fun parse(path: String): ResourceKeys {
        val document = DocumentBuilderFactory.newInstance()
            .newDocumentBuilder()
            .parse(resFile(path))
        val result = linkedMapOf<String, Set<String>>()
        val root = document.documentElement
        for (index in 0 until root.childNodes.length) {
            val node = root.childNodes.item(index)
            if (node !is Element) continue
            when (node.tagName) {
                "string" -> {
                    val name = node.getAttribute("name")
                    assertTrue(
                        result.put("string:$name", placeholders(node.textContent)) == null,
                        "duplicate string resource: $name",
                    )
                }

                "plurals" -> {
                    val name = node.getAttribute("name")
                    val union = mutableSetOf<String>()
                    val items = node.getElementsByTagName("item")
                    for (itemIndex in 0 until items.length) {
                        val item = items.item(itemIndex) as Element
                        union += placeholders(item.textContent)
                    }
                    assertTrue(
                        result.put("plurals:$name", union) == null,
                        "duplicate plurals resource: $name",
                    )
                }
            }
        }
        return ResourceKeys(result)
    }

    @Test
    fun english_and_chinese_define_the_same_keys_and_placeholders() {
        val en = parse("values/strings.xml")
        val zh = parse("values-zh-rCN/strings.xml")

        assertEquals(
            en.names,
            zh.names,
            "default and zh-CN resource keys differ",
        )
        for (name in en.names.sorted()) {
            assertEquals(
                en.placeholders.getValue(name),
                zh.placeholders.getValue(name),
                "format placeholders differ for $name",
            )
        }
    }
}
