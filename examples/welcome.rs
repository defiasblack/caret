fn main() {
    let editor = "Caret";
    println!("Welcome to {editor}!");

    // Press F1 inside the editor for the full key map.
    for feature in ["modes", "search", "undo", "syntax color"] {
        println!("✓ {feature}");
    }
}
