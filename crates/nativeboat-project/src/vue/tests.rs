use super::*;

#[test]
fn components_split_into_their_top_level_blocks() {
    let source = "<!-- <script>not a block</script> -->\n\
                  <template>\n  <div v-if=\"a > b\"><template #slot><b>x</b></template></div>\n</template>\n\n\
                  <script setup lang=\"TS\">\nimport A from './A.vue'\n</script>\n\
                  <style scoped lang='scss'>\n.a { color: red }\n</style>\n\
                  <style src=\"./theme.css\"></style>\n<i18n>{}</i18n>\n<script/>\n";
    let blocks = blocks(source);
    let kinds: Vec<BlockKind> = blocks.iter().map(|block| block.kind).collect();
    assert_eq!(
        kinds,
        vec![
            BlockKind::Template,
            BlockKind::Script,
            BlockKind::Style,
            BlockKind::Style,
            BlockKind::Custom,
        ]
    );
    assert_eq!(
        blocks[0].content,
        "\n  <div v-if=\"a > b\"><template #slot><b>x</b></template></div>\n"
    );
    assert_eq!(blocks[0].line(source), 2);
    assert_eq!(blocks[1].content, "\nimport A from './A.vue'\n");
    assert_eq!(blocks[1].lang.as_deref(), Some("ts"));
    assert_eq!(blocks[1].line(source), 6);
    assert_eq!(blocks[2].lang.as_deref(), Some("scss"));
    assert!(blocks[2].scoped && !blocks[3].scoped && !blocks[1].scoped);
    assert_eq!(blocks[3].src.as_deref(), Some("./theme.css"));
    assert_eq!(blocks[3].content, "");
}

#[test]
fn unclosed_blocks_end_the_component() {
    let blocks = blocks("<template><div></div></template><script>let a = 1");
    assert_eq!(blocks.len(), 1);
    assert!(super::blocks("a < b").is_empty());
}
