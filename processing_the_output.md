PRD for processing the output of the notes rendering process.

1. It should be interactive - there should be a button to send the annotations back to the model for updates in place
2. They should operate on the lowest bandwith possible
    - first try to send the annotations with metadata only
        - the server tries to ocr the annotations and preprocess the context based on the metadata
        - the server also receives the png/pdf of the annotation for wider context if needed
3. The ocr on the server should run locally and use native rust/python libraries to do so - the model can then decide to reach for more detailed information if needed
    - the best option would be to use rust for it
4. The process of sending feedback and waiting for the review should be reactive
    - we should receive information about the server progress and see the in progress updates in the UI
    - the new version should be rendered for us
5. The document sent back after the review should be able to retain the annotations which were linked to the sections in the original document if applicable
6. The server should be responsible for keeping the converstaion between the ai agent, the user and the document reader - consider how to pass requests for updates to the model and how to receive information back when the model is ready to ship the delta for the document update
7. implement the update/refresh flow
