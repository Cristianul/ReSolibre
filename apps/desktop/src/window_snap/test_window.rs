//! Owned native windows shared by Linux and Windows placement regressions.

slint::slint! {
    export component SnapTestWindow inherits Window {
        title: "ReSolibre snap verification";
        in property <bool> frameless: true;
        no-frame: root.frameless;
        resize-border-width: 6px;
        min-width: 220px;
        min-height: 160px;
        preferred-width: 220px;
        preferred-height: 160px;
        Text { text: "ReSolibre · window snap test"; }
    }
}
